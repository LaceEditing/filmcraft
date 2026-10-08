//! NVIDIA hardware codecs on Linux: NVDEC decoding ([`nvdec`]) and NVENC encoding ([`nvenc`])
//! through the driver's CUDA, nvcuvid and nvEncodeAPI libraries, loaded at run time
//! ([`cuda`]). The bindings ([`sys`]) are generated from FFmpeg's MIT-licensed
//! `nv-codec-headers`. Nothing here is required: without the driver, a GPU or a supported
//! stream, FilmCraft's own codecs are used, and a failure mid-stream falls back to them too.

#[allow(unsafe_code)]
pub(crate) mod cuda;
#[allow(unsafe_code)]
pub(crate) mod nvdec;
#[allow(unsafe_code)]
pub(crate) mod nvenc;
// Generated declarations: bindgen's bitfield accessors carry `unsafe` blocks of its own making,
// the one exception to the SAFETY-comment rule (ADR 0001); they are never edited by hand.
#[allow(unsafe_code, non_camel_case_types, non_snake_case, non_upper_case_globals, dead_code, clippy::all, clippy::undocumented_unsafe_blocks, unused)]
pub(crate) mod sys;

use crate::hybrid::HybridDecoder;

/// The NVDEC factory: a [`HybridDecoder`] around [`nvdec::NvDecoder`] for `avcC` / `hvcC`
/// streams the GPU decodes, `None` otherwise.
pub(crate) fn nvdec_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match nvdec::NvDecoder::new(info.clone()) {
        Ok(d) => Some(Ok(Box::new(HybridDecoder::new(Box::new(d), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// Whether the NVIDIA driver and a GPU with NVDEC are present (loads the libraries once).
pub(crate) fn available() -> Result<String, String> {
    let cuda = cuda::Cuda::get()?;
    nvdec::Cuvid::get()?;
    Ok(cuda.device_name.clone())
}

/// Whether the GPU decodes the stream of `entry` (whatever the Hardware decoding setting says).
pub(crate) fn decodes(entry: &filmcraft_isobmff::SampleEntry) -> bool {
    let Some(Ok(info)) = filmcraft_codecs::hw::NalStreamInfo::from_entry(entry) else { return false };
    let (Ok(cuda), Ok(cuvid)) = (cuda::Cuda::get(), nvdec::Cuvid::get()) else { return false };
    nvdec::supports(&cuda, &cuvid, &info).is_ok()
}
