//! OS media integration (layer L5): hardware video decoding through the operating system's codecs.
//!
//! [`register`] puts the platform's hardware decoder factory in front of FilmCraft's own decoders
//! (`filmcraft_codecs::register_video_decoder`). Today that is VideoToolbox on macOS for H.264
//! (`avcC`) and HEVC (`hvcC`) streams, 8- and 10-bit, 4:2:0 and 4:2:2; on other systems
//! registration does nothing and reports [`Availability::Unavailable`].
//!
//! Hardware decoding never makes a file undecodable:
//!
//! - the factory declines (falls through to the software decoder) when Settings ▸ Playback ▸
//!   Hardware decoding is Off (`filmcraft_codecs::hw::set_hardware_decoding`), when the stream's
//!   format is one the hardware path does not take, or when the OS cannot create a hardware
//!   session for it (profile, size, no hardware decoder);
//! - a decoder that fails mid-stream switches to the software decoder transparently
//!   ([`HybridDecoder`]) and logs it.
//!
//! Pictures are the software decoder's: same planes (bit-exact on the parity fixtures), colour,
//! pixel aspect, pts and presentation order, so the two are interchangeable.
//!
//! This is the one crate allowed to use `unsafe` (OS FFI), and only in its FFI modules
//! (docs/adr/0001-platform-ffi.md).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod hybrid;
#[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
mod nvidia;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod videotoolbox;

pub use hybrid::HybridDecoder;

/// Whether exports may use a hardware H.264 encoder (NVENC on Linux); on by default.
pub fn hardware_encoding() -> bool {
    #[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
    {
        nvidia::nvenc::hardware_encoding()
    }
    #[cfg(not(all(target_os = "linux", not(target_arch = "wasm32"))))]
    {
        false
    }
}

/// Turn hardware H.264 encoding for exports on or off (no effect where there is none).
pub fn set_hardware_encoding(on: bool) {
    #[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
    {
        nvidia::nvenc::set_hardware_encoding(on)
    }
    #[cfg(not(all(target_os = "linux", not(target_arch = "wasm32"))))]
    {
        let _ = on;
    }
}

/// What [`register`] made available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A hardware decoder factory was registered (its name).
    Available(&'static str),
    /// Nothing to register on this system (why).
    Unavailable(&'static str),
}

/// Register the platform's hardware video decoders (call once at startup; repeated calls are
/// harmless). Streams they do not take, and every stream while hardware decoding is Off, keep
/// using FilmCraft's own decoders.
pub fn register() -> Availability {
    #[cfg(target_os = "macos")]
    {
        filmcraft_codecs::register_video_decoder(videotoolbox_factory);
        Availability::Available("VideoToolbox")
    }
    #[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
    {
        match nvidia::available() {
            Ok(_) => {
                filmcraft_codecs::register_video_decoder(nvidia::nvdec_factory);
                filmcraft_export::register_encoder(nvidia::nvenc::nvenc_factory);
                Availability::Available("NVDEC")
            }
            Err(why) => {
                log::info!("no hardware video decoder: {why}");
                Availability::Unavailable("no NVIDIA GPU with NVDEC (the NVIDIA driver is not installed or no GPU was found)")
            }
        }
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", not(target_arch = "wasm32")))))]
    {
        Availability::Unavailable("no hardware video decoder for this system yet")
    }
}

/// Whether [`register`] has put a hardware decoder factory in front of our decoders.
pub fn registered() -> bool {
    #[cfg(target_os = "macos")]
    {
        filmcraft_codecs::video_decoder_registered(videotoolbox_factory)
    }
    #[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
    {
        filmcraft_codecs::video_decoder_registered(nvidia::nvdec_factory)
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", not(target_arch = "wasm32")))))]
    {
        false
    }
}

/// Whether this system's hardware decoder takes the stream of `entry` (whatever the Hardware
/// decoding setting says): diagnostics and tests.
pub fn hardware_decoder_for(entry: &filmcraft_isobmff::SampleEntry) -> bool {
    #[cfg(target_os = "macos")]
    {
        filmcraft_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| videotoolbox::VtDecoder::new(info).is_ok())
    }
    #[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
    {
        nvidia::decodes(entry)
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", not(target_arch = "wasm32")))))]
    {
        let _ = entry;
        false
    }
}

/// The VideoToolbox factory: a [`HybridDecoder`] around [`videotoolbox::VtDecoder`] for `avcC` /
/// `hvcC` streams VideoToolbox can decode in hardware, `None` otherwise.
#[cfg(target_os = "macos")]
pub fn videotoolbox_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match videotoolbox::VtDecoder::new(info.clone()) {
        Ok(vt) => Some(Ok(Box::new(HybridDecoder::new(Box::new(vt), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}
