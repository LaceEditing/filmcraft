//! NVDEC (nvcuvid) hardware H.264 / HEVC decoding on Linux.
//!
//! Rules as for VideoToolbox: every `unsafe` block has a `// SAFETY:` comment; no panic may unwind
//! into the driver (the parser callbacks run under `catch_unwind`); every failure is a `Result`
//! the caller ([`crate::hybrid::HybridDecoder`]) answers by switching to the software decoder.
//!
//! A decoder is a bitstream parser (`cuvidCreateVideoParser`) that calls back on this thread
//! from inside `cuvidParseVideoData`: once with the stream format (the decoder is created then,
//! sized from the sample entry), once per picture to decode, and once per picture to display, in
//! presentation order. Length-prefixed MP4 samples are fed as Annex B NAL units, the parameter
//! sets of the sample entry first. Displayed pictures are mapped (NV12 / P016 in device memory),
//! copied to host memory and split into planar [`PixelData::Yuv8`] / [`PixelData::Yuv16`].

use std::ffi::{c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, Result, VideoDecoder};
use filmcraft_frame::{Chroma, PixelData, VideoFrame, pool};

use super::cuda::{Cuda, CudaError, sym};
use super::sys;

/// Pictures the parser may hold back for reordering before display.
const DISPLAY_DELAY: u32 = 4;
/// Decode surfaces beyond the stream's minimum, so the parser never stalls.
const EXTRA_SURFACES: u32 = 4;

/// The nvcuvid library.
pub(crate) struct Cuvid {
    _lib: libloading::Library,
    create_parser: sys::tcuvidCreateVideoParser,
    parse: sys::tcuvidParseVideoData,
    destroy_parser: sys::tcuvidDestroyVideoParser,
    create_decoder: sys::tcuvidCreateDecoder,
    destroy_decoder: sys::tcuvidDestroyDecoder,
    decode_picture: sys::tcuvidDecodePicture,
    map_frame: sys::tcuvidMapVideoFrame64,
    unmap_frame: sys::tcuvidUnmapVideoFrame64,
    get_caps: sys::tcuvidGetDecoderCaps,
}

// SAFETY: function pointers into a library kept loaded for the struct's life; handles are not
// stored here.
unsafe impl Send for Cuvid {}
// SAFETY: as above.
unsafe impl Sync for Cuvid {}

static CUVID: OnceLock<std::result::Result<Arc<Cuvid>, String>> = OnceLock::new();

impl Cuvid {
    pub(crate) fn get() -> std::result::Result<Arc<Cuvid>, String> {
        CUVID.get_or_init(Self::load).clone()
    }

    fn load() -> std::result::Result<Arc<Cuvid>, String> {
        // SAFETY: loading the NVIDIA video decoding library; kept for the program's life.
        let lib = unsafe { libloading::Library::new("libnvcuvid.so.1") }.map_err(|e| format!("no NVDEC library (libnvcuvid.so.1): {e}"))?;
        let s = Cuvid {
            create_parser: sym!(lib, "cuvidCreateVideoParser", sys::tcuvidCreateVideoParser).flatten(),
            parse: sym!(lib, "cuvidParseVideoData", sys::tcuvidParseVideoData).flatten(),
            destroy_parser: sym!(lib, "cuvidDestroyVideoParser", sys::tcuvidDestroyVideoParser).flatten(),
            create_decoder: sym!(lib, "cuvidCreateDecoder", sys::tcuvidCreateDecoder).flatten(),
            destroy_decoder: sym!(lib, "cuvidDestroyDecoder", sys::tcuvidDestroyDecoder).flatten(),
            decode_picture: sym!(lib, "cuvidDecodePicture", sys::tcuvidDecodePicture).flatten(),
            map_frame: sym!(lib, "cuvidMapVideoFrame64", sys::tcuvidMapVideoFrame64).flatten(),
            unmap_frame: sym!(lib, "cuvidUnmapVideoFrame64", sys::tcuvidUnmapVideoFrame64).flatten(),
            get_caps: sym!(lib, "cuvidGetDecoderCaps", sys::tcuvidGetDecoderCaps).flatten(),
            _lib: lib,
        };
        if s.create_parser.is_none()
            || s.parse.is_none()
            || s.destroy_parser.is_none()
            || s.create_decoder.is_none()
            || s.destroy_decoder.is_none()
            || s.decode_picture.is_none()
            || s.map_frame.is_none()
            || s.unmap_frame.is_none()
            || s.get_caps.is_none()
        {
            return Err("the NVDEC library lacks the nvcuvid API".into());
        }
        Ok(Arc::new(s))
    }
}

fn codec_of(info: &NalStreamInfo) -> sys::cudaVideoCodec {
    match info.codec {
        NalCodec::H264 => sys::cudaVideoCodec_H264,
        NalCodec::Hevc => sys::cudaVideoCodec_HEVC,
    }
}

/// Whether this GPU decodes streams like `info` (codec, chroma, bit depth, size).
pub(crate) fn supports(cuda: &Cuda, cuvid: &Cuvid, info: &NalStreamInfo) -> std::result::Result<(), String> {
    if info.chroma_format_idc != 1 {
        return Err("only 4:2:0 streams are decoded in hardware".into());
    }
    if info.interlaced {
        return Err("interlaced streams are decoded in software".into());
    }
    if info.bit_depth_luma != info.bit_depth_chroma || !(8..=12).contains(&info.bit_depth_luma) {
        return Err(format!("{}-bit streams are decoded in software", info.bit_depth_luma));
    }
    let Some(get_caps) = cuvid.get_caps else { return Err("no caps query".into()) };
    let mut caps = sys::CUVIDDECODECAPS {
        eCodecType: codec_of(info),
        eChromaFormat: sys::cudaVideoChromaFormat_420,
        nBitDepthMinus8: info.bit_depth_luma - 8,
        ..Default::default()
    };
    // SAFETY: a plain query with a valid out-struct; needs a current context.
    let r = cuda.scope(|| unsafe { get_caps(&mut caps) }).map_err(|e| e.to_string())?;
    cuda.check("cuvidGetDecoderCaps", r).map_err(|e| e.to_string())?;
    if caps.bIsSupported == 0 {
        return Err("this GPU has no hardware decoder for the stream".into());
    }
    let (w, h) = info.coded;
    if w < caps.nMinWidth as u32 || h < caps.nMinHeight as u32 || w > caps.nMaxWidth || h > caps.nMaxHeight {
        return Err(format!("{w}x{h} is outside the hardware decoder's range"));
    }
    if (w.div_ceil(16) * h.div_ceil(16)) > caps.nMaxMBCount {
        return Err("the picture has more macroblocks than the hardware decoder takes".into());
    }
    let want = if info.bit_depth_luma > 8 { 1u16 << sys::cudaVideoSurfaceFormat_P016 } else { 1u16 << sys::cudaVideoSurfaceFormat_NV12 };
    if caps.nOutputFormatMask & want == 0 {
        return Err("the hardware decoder cannot output this pixel format".into());
    }
    Ok(())
}

/// State shared with the parser callbacks (their `pUserData`): boxed by the decoder, kept alive
/// until the parser is destroyed.
struct Shared {
    cuda: Arc<Cuda>,
    cuvid: Arc<Cuvid>,
    info: NalStreamInfo,
    decoder: Mutex<Option<sys::CUvideodecoder>>,
    /// Luma size the decoder was created with, and its surface format.
    layout: Mutex<Option<Layout>>,
    out: Mutex<Vec<Output>>,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    /// Output picture origin (luma samples) inside the decoded surface.
    origin: (u32, u32),
    /// Output picture size.
    size: (u32, u32),
    /// Rows of the surface the chroma plane starts after.
    luma_rows: u32,
    bits: u32,
}

enum Output {
    Frame(DecodedFrame),
    Failed(String),
}

impl Shared {
    fn fail(&self, why: impl Into<String>) {
        self.out.lock().unwrap_or_else(PoisonError::into_inner).push(Output::Failed(why.into()));
    }
}

/// Called by the parser with the stream's format: create (or keep) the decoder. Returns the number
/// of decode surfaces, 0 on failure.
unsafe extern "C" fn on_sequence(user: *mut c_void, format: *mut sys::CUVIDEOFORMAT) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user` is the `Shared` the decoder registered and keeps alive; `format` is the
        // parser's struct for the duration of the callback.
        let (s, f) = unsafe { (&*(user as *const Shared), &*format) };
        sequence(s, f)
    }));
    match r {
        Ok(Ok(n)) => n as c_int,
        Ok(Err(e)) => {
            // SAFETY: as above.
            unsafe { &*(user as *const Shared) }.fail(e);
            0
        }
        Err(_) => 0,
    }
}

fn sequence(s: &Shared, f: &sys::CUVIDEOFORMAT) -> std::result::Result<u32, String> {
    let bits = f.bit_depth_luma_minus8 as u32 + 8;
    if f.chroma_format != sys::cudaVideoChromaFormat_420 {
        return Err("the stream is not 4:2:0".into());
    }
    let surfaces = f.min_num_decode_surfaces as u32 + EXTRA_SURFACES;
    let (cw, ch) = (f.coded_width, f.coded_height);
    let (l, t, r, b) = (f.display_area.left, f.display_area.top, f.display_area.right, f.display_area.bottom);
    let (ow, oh) = ((r - l).max(0) as u32, (b - t).max(0) as u32);
    if ow == 0 || oh == 0 || cw == 0 || ch == 0 {
        return Err("empty picture".into());
    }
    let mut dec = s.decoder.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(old) = dec.take()
        && let Some(destroy) = s.cuvid.destroy_decoder
    {
        // SAFETY: a decoder handle this struct created; destroyed once.
        unsafe { destroy(old) };
    }
    let info = sys::CUVIDDECODECREATEINFO {
        ulWidth: cw as sys::tcu_ulong,
        ulHeight: ch as sys::tcu_ulong,
        ulNumDecodeSurfaces: surfaces as sys::tcu_ulong,
        CodecType: f.codec,
        ChromaFormat: f.chroma_format,
        ulCreationFlags: sys::cudaVideoCreate_PreferCUVID as sys::tcu_ulong,
        bitDepthMinus8: f.bit_depth_luma_minus8 as sys::tcu_ulong,
        ulIntraDecodeOnly: 0,
        ulMaxWidth: cw as sys::tcu_ulong,
        ulMaxHeight: ch as sys::tcu_ulong,
        Reserved1: 0,
        display_area: sys::_CUVIDDECODECREATEINFO__bindgen_ty_1 { left: l as i16, top: t as i16, right: r as i16, bottom: b as i16 },
        OutputFormat: if bits > 8 { sys::cudaVideoSurfaceFormat_P016 } else { sys::cudaVideoSurfaceFormat_NV12 },
        DeinterlaceMode: sys::cudaVideoDeinterlaceMode_Weave,
        ulTargetWidth: cw as sys::tcu_ulong,
        ulTargetHeight: ch as sys::tcu_ulong,
        ulNumOutputSurfaces: 2,
        vidLock: std::ptr::null_mut(),
        target_rect: sys::_CUVIDDECODECREATEINFO__bindgen_ty_2 { left: 0, top: 0, right: cw as i16, bottom: ch as i16 },
        enableHistogram: 0,
        enableDecodeFeatures: 0,
        Reserved2: [0; 3],
    };
    let mut info = info;
    let Some(create) = s.cuvid.create_decoder else { return Err("no decoder".into()) };
    let mut handle: sys::CUvideodecoder = std::ptr::null_mut();
    // SAFETY: valid out-handle and a fully initialised create-info; the context is current (we are
    // inside `cuvidParseVideoData`, called under `Cuda::scope`).
    let r = unsafe { create(&mut handle, &mut info) };
    s.cuda.check("cuvidCreateDecoder", r).map_err(|e| e.to_string())?;
    if handle.is_null() {
        return Err("cuvidCreateDecoder returned no decoder".into());
    }
    *dec = Some(handle);
    *s.layout.lock().unwrap_or_else(PoisonError::into_inner) = Some(Layout { origin: (l.max(0) as u32, t.max(0) as u32), size: (ow, oh), luma_rows: ch, bits });
    Ok(surfaces)
}

/// Called by the parser with a picture to decode.
unsafe extern "C" fn on_decode(user: *mut c_void, pic: *mut sys::CUVIDPICPARAMS) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `on_sequence`.
        let s = unsafe { &*(user as *const Shared) };
        let dec = s.decoder.lock().unwrap_or_else(PoisonError::into_inner);
        let (Some(handle), Some(decode)) = (*dec, s.cuvid.decode_picture) else {
            s.fail("picture before the stream format");
            return 0;
        };
        // SAFETY: the parser's picture parameters, valid for the callback; a live decoder handle.
        let r = unsafe { decode(handle, pic) };
        if r != sys::CUDA_SUCCESS {
            s.fail(s.cuda.error("cuvidDecodePicture", r).to_string());
            return 0;
        }
        1
    }));
    r.unwrap_or(0)
}

/// Called by the parser with a decoded picture to display (presentation order).
unsafe extern "C" fn on_display(user: *mut c_void, disp: *mut sys::CUVIDPARSERDISPINFO) -> c_int {
    let r = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `on_sequence`.
        let (s, d) = unsafe { (&*(user as *const Shared), &*disp) };
        match display(s, d) {
            Ok(frame) => {
                s.out.lock().unwrap_or_else(PoisonError::into_inner).push(Output::Frame(frame));
                1
            }
            Err(e) => {
                s.fail(e);
                0
            }
        }
    }));
    r.unwrap_or(0)
}

fn display(s: &Shared, d: &sys::CUVIDPARSERDISPINFO) -> std::result::Result<DecodedFrame, String> {
    let layout = s.layout.lock().unwrap_or_else(PoisonError::into_inner).ok_or("display before the stream format")?;
    let handle = s.decoder.lock().unwrap_or_else(PoisonError::into_inner).ok_or("display without a decoder")?;
    let (Some(map), Some(unmap)) = (s.cuvid.map_frame, s.cuvid.unmap_frame) else { return Err("no frame mapping".into()) };
    let mut proc_params = sys::CUVIDPROCPARAMS { progressive_frame: d.progressive_frame, top_field_first: d.top_field_first, ..Default::default() };
    let mut dptr: u64 = 0;
    let mut pitch: u32 = 0;
    // SAFETY: a live decoder handle and the picture index the parser just gave us; out-pointers to
    // locals. The surface stays mapped until `unmap` below.
    let r = unsafe { map(handle, d.picture_index, &mut dptr, &mut pitch, &mut proc_params) };
    s.cuda.check("cuvidMapVideoFrame", r).map_err(|e| e.to_string())?;
    let copied = copy_planes(&s.cuda, &layout, dptr, pitch as usize);
    // SAFETY: unmaps what `map` mapped, exactly once.
    let r = unsafe { unmap(handle, dptr) };
    let data = copied?;
    s.cuda.check("cuvidUnmapVideoFrame", r).map_err(|e| e.to_string())?;
    let (w, h) = layout.size;
    let frame = VideoFrame { width: w, height: h, data, color: s.info.color, par: s.info.par, pts: filmcraft_time::Tick(d.timestamp) };
    Ok(DecodedFrame { pts: d.timestamp, frame, draft: false })
}

/// Copy the mapped NV12 / P016 surface into planar host memory, cropped to the output picture.
fn copy_planes(cuda: &Cuda, l: &Layout, dptr: u64, pitch: usize) -> std::result::Result<PixelData, String> {
    let (w, h) = (l.size.0 as usize, l.size.1 as usize);
    let (ox, oy) = (l.origin.0 as usize, l.origin.1 as usize);
    let bps = if l.bits > 8 { 2 } else { 1 };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (cox, coy) = (ox / 2, oy / 2);
    if pitch < (ox + w) * bps {
        return Err("mapped surface narrower than the picture".into());
    }
    let row_bytes = w * bps;
    let mut luma = vec![0u8; row_bytes * h];
    let luma_src = dptr.checked_add((oy * pitch + ox * bps) as u64).ok_or("surface address overflow")?;
    cuda.copy_rows_to_host(luma_src, pitch, row_bytes, h, &mut luma).map_err(|e| e.to_string())?;
    let chroma_row_bytes = cw * 2 * bps;
    let mut chroma = vec![0u8; chroma_row_bytes * ch];
    let chroma_src = dptr.checked_add(((l.luma_rows as usize + coy) * pitch + cox * 2 * bps) as u64).ok_or("surface address overflow")?;
    cuda.copy_rows_to_host(chroma_src, pitch, chroma_row_bytes, ch, &mut chroma).map_err(|e| e.to_string())?;
    Ok(if bps == 1 {
        let mut y = pool::take_u8(w * h);
        y.extend_from_slice(&luma);
        let (mut u, mut v) = (pool::take_u8(cw * ch), pool::take_u8(cw * ch));
        for pair in chroma.as_chunks::<2>().0 {
            u.push(pair[0]);
            v.push(pair[1]);
        }
        PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None }
    } else {
        // P016: samples in the high bits of 16-bit words
        let shift = 16 - l.bits;
        let mut y = pool::take_u16(w * h);
        y.extend(luma.as_chunks::<2>().0.iter().map(|b| u16::from_ne_bytes(*b) >> shift));
        let (mut u, mut v) = (pool::take_u16(cw * ch), pool::take_u16(cw * ch));
        for q in chroma.as_chunks::<4>().0 {
            u.push(u16::from_ne_bytes([q[0], q[1]]) >> shift);
            v.push(u16::from_ne_bytes([q[2], q[3]]) >> shift);
        }
        PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, bits: l.bits, alpha: None }
    })
}

/// An NVDEC decoder for one stream.
pub(crate) struct NvDecoder {
    shared: Box<Shared>,
    parser: sys::CUvideoparser,
    /// Annex B buffer reused per sample.
    buf: Vec<u8>,
    /// Parameter sets go out before the next sample (at start and after a reset).
    need_headers: bool,
    /// Pictures displayed by the callbacks, not yet returned.
    pending: Vec<DecodedFrame>,
}

// SAFETY: the parser is only ever used from the thread holding `&mut self`; the driver permits
// moving it between threads.
unsafe impl Send for NvDecoder {}

impl NvDecoder {
    /// A decoder for `info`, declined (`Err`) when the GPU cannot take the stream.
    pub(crate) fn new(info: NalStreamInfo) -> std::result::Result<Self, String> {
        let cuda = Cuda::get()?;
        let cuvid = Cuvid::get()?;
        supports(&cuda, &cuvid, &info)?;
        let shared = Box::new(Shared {
            cuda: cuda.clone(),
            cuvid: cuvid.clone(),
            info,
            decoder: Mutex::new(None),
            layout: Mutex::new(None),
            out: Mutex::new(Vec::new()),
        });
        let parser = Self::create_parser(&shared)?;
        Ok(Self { shared, parser, buf: Vec::new(), need_headers: true, pending: Vec::new() })
    }

    fn create_parser(shared: &Shared) -> std::result::Result<sys::CUvideoparser, String> {
        let Some(create) = shared.cuvid.create_parser else { return Err("no parser".into()) };
        let mut params = sys::CUVIDPARSERPARAMS {
            CodecType: codec_of(&shared.info),
            ulMaxNumDecodeSurfaces: 1,
            ulClockRate: 0,
            ulErrorThreshold: 100,
            ulMaxDisplayDelay: DISPLAY_DELAY,
            pUserData: shared as *const Shared as *mut c_void,
            pfnSequenceCallback: Some(on_sequence),
            pfnDecodePicture: Some(on_decode),
            pfnDisplayPicture: Some(on_display),
            ..Default::default()
        };
        let mut parser: sys::CUvideoparser = std::ptr::null_mut();
        // SAFETY: valid out-handle and parameters; the callbacks' user data outlives the parser
        // (`Shared` is boxed and dropped after `cuvidDestroyVideoParser`).
        let r = shared.cuda.scope(|| unsafe { create(&mut parser, &mut params) }).map_err(|e| e.to_string())?;
        shared.cuda.check("cuvidCreateVideoParser", r).map_err(|e| e.to_string())?;
        if parser.is_null() {
            return Err("cuvidCreateVideoParser returned no parser".into());
        }
        Ok(parser)
    }

    fn destroy_parser(&mut self) {
        if !self.parser.is_null()
            && let Some(destroy) = self.shared.cuvid.destroy_parser
        {
            // SAFETY: a parser handle this struct created; destroyed once, context current.
            let _ = self.shared.cuda.scope(|| unsafe { destroy(self.parser) });
            self.parser = std::ptr::null_mut();
        }
    }

    fn destroy_decoder(&mut self) {
        let mut dec = self.shared.decoder.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(h) = dec.take()
            && let Some(destroy) = self.shared.cuvid.destroy_decoder
        {
            // SAFETY: a decoder handle created in `sequence`; destroyed once, context current.
            let _ = self.shared.cuda.scope(|| unsafe { destroy(h) });
        }
    }

    /// Feed one packet (Annex B bytes) to the parser.
    fn send(&mut self, payload: &[u8], flags: u32, pts: i64) -> Result<()> {
        let Some(parse) = self.shared.cuvid.parse else { return Err(CodecError::Decode("no parser".into())) };
        let mut packet = sys::CUVIDSOURCEDATAPACKET {
            flags: flags as sys::tcu_ulong,
            payload_size: payload.len() as sys::tcu_ulong,
            payload: payload.as_ptr(),
            timestamp: pts,
        };
        let parser = self.parser;
        // SAFETY: a live parser; the packet points at `payload`, alive for the call.
        let r = self.shared.cuda.scope(|| unsafe { parse(parser, &mut packet) }).map_err(|e: CudaError| CodecError::Decode(e.to_string()))?;
        self.shared.cuda.check("cuvidParseVideoData", r).map_err(|e| CodecError::Decode(e.to_string()))?;
        self.collect()
    }

    /// Frames the callbacks produced, or the failure they recorded.
    fn collect(&mut self) -> Result<()> {
        let out = std::mem::take(&mut *self.shared.out.lock().unwrap_or_else(PoisonError::into_inner));
        let mut frames = Vec::new();
        let mut failed = None;
        for o in out {
            match o {
                Output::Frame(f) => frames.push(f),
                Output::Failed(e) => failed = Some(e),
            }
        }
        self.pending.extend(frames);
        match failed {
            Some(e) => Err(CodecError::Decode(e)),
            None => Ok(()),
        }
    }

    fn annex_b(&mut self, sample: &[u8]) {
        self.buf.clear();
        if self.need_headers {
            for ps in &self.shared.info.parameter_sets {
                self.buf.extend_from_slice(&[0, 0, 0, 1]);
                self.buf.extend_from_slice(ps);
            }
            self.need_headers = false;
        }
        for nal in self.shared.info.nals(sample) {
            self.buf.extend_from_slice(&[0, 0, 0, 1]);
            self.buf.extend_from_slice(nal);
        }
    }
}

impl Drop for NvDecoder {
    fn drop(&mut self) {
        self.destroy_parser();
        self.destroy_decoder();
    }
}

impl VideoDecoder for NvDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        self.annex_b(sample);
        let buf = std::mem::take(&mut self.buf);
        let r = self.send(&buf, sys::CUVID_PKT_TIMESTAMP | sys::CUVID_PKT_ENDOFPICTURE, pts);
        self.buf = buf;
        r?;
        Ok(std::mem::take(&mut self.pending))
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        let _ = self.send(&[], sys::CUVID_PKT_ENDOFSTREAM, 0);
        // the parser is finished after an end of stream: a fresh one for what follows
        self.destroy_parser();
        if let Ok(p) = Self::create_parser(&self.shared) {
            self.parser = p;
        }
        self.need_headers = true;
        std::mem::take(&mut self.pending)
    }

    fn reset(&mut self) {
        self.destroy_parser();
        self.shared.out.lock().unwrap_or_else(PoisonError::into_inner).clear();
        self.pending.clear();
        if let Ok(p) = Self::create_parser(&self.shared) {
            self.parser = p;
        }
        self.need_headers = true;
    }

    fn name(&self) -> &str {
        match self.shared.info.codec {
            NalCodec::H264 => "NVDEC H.264",
            NalCodec::Hevc => "NVDEC HEVC",
        }
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.shared.info.is_random_access(sample)
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.shared.info.is_disposable(sample)
    }
}
