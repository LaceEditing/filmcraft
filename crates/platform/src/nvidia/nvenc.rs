//! NVENC hardware H.264 encoding for exports (Linux).
//!
//! Rules as for the decoder: every `unsafe` block has a `// SAFETY:` comment; every failure is a
//! `Result`. The encoder is created by [`nvenc_factory`], which the export crate asks first; when
//! the GPU, the driver or the settings rule it out, the factory answers `None` and FilmCraft's own
//! H.264 encoder takes over, so an export never fails for want of hardware.
//!
//! Frames arrive as sRGB RGBA8 and are converted to BT.709 limited-range NV12 on the CPU with the
//! same arithmetic as the software path, so hardware and software exports match in colour. NVENC
//! returns Annex B access units in decoding order; SPS / PPS go into the `avcC` of the sample
//! entry, the rest becomes length-prefixed MP4 samples.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use filmcraft_export::{BitrateMode, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format, H264Pass, H264Profile, VideoEncoder};
use filmcraft_isobmff::SampleEntry;
use filmcraft_time::FrameRate;

use super::cuda::{Cuda, sym};
use super::sys;

/// B-frames between references; 2 is what the software encoder's defaults produce.
const B_FRAMES: u32 = 2;
/// Input / output buffers in flight: the B-frame delay plus headroom.
const BUFFERS: usize = 8;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Whether exports may use the GPU's H.264 encoder (on by default; `FILMCRAFT_NO_NVENC` in the
/// environment turns it off for a session).
pub fn hardware_encoding() -> bool {
    ENABLED.load(Ordering::Relaxed) && std::env::var_os("FILMCRAFT_NO_NVENC").is_none()
}

pub fn set_hardware_encoding(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

type CreateInstance = unsafe extern "C" fn(*mut sys::NV_ENCODE_API_FUNCTION_LIST) -> sys::NVENCSTATUS;
type MaxVersion = unsafe extern "C" fn(*mut u32) -> sys::NVENCSTATUS;

const fn struct_version(v: u32) -> u32 {
    (sys::NVENCAPI_MAJOR_VERSION | (sys::NVENCAPI_MINOR_VERSION << 24)) | (v << 16) | (0x7 << 28)
}
const API_VERSION: u32 = sys::NVENCAPI_MAJOR_VERSION | (sys::NVENCAPI_MINOR_VERSION << 24);
const FUNCTION_LIST_VER: u32 = struct_version(2);
const OPEN_SESSION_VER: u32 = struct_version(1);
const PRESET_CONFIG_VER: u32 = struct_version(5) | (1 << 31);
const CONFIG_VER: u32 = struct_version(9) | (1 << 31);
const INITIALIZE_PARAMS_VER: u32 = struct_version(7) | (1 << 31);
const CREATE_INPUT_BUFFER_VER: u32 = struct_version(2);
const CREATE_BITSTREAM_BUFFER_VER: u32 = struct_version(1);
const LOCK_INPUT_BUFFER_VER: u32 = struct_version(1);
const LOCK_BITSTREAM_VER: u32 = struct_version(2) | (1 << 31);
const PIC_PARAMS_VER: u32 = struct_version(7) | (1 << 31);

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> sys::GUID {
    sys::GUID { Data1: d1, Data2: d2, Data3: d3, Data4: d4 }
}
// GUIDs as published in nvEncodeAPI.h.
const CODEC_H264: sys::GUID = guid(0x6bc82762, 0x4e63, 0x4ca4, [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf]);
const PRESET_P4: sys::GUID = guid(0x90a7b826, 0xdf06, 0x4862, [0xb9, 0xd2, 0xcd, 0x6d, 0x73, 0xa0, 0x86, 0x81]);
const PROFILE_BASELINE: sys::GUID = guid(0x0727bcaa, 0x78c4, 0x4c83, [0x8c, 0x2f, 0xef, 0x3d, 0xff, 0x26, 0x7c, 0x6a]);
const PROFILE_MAIN: sys::GUID = guid(0x60b5c1d4, 0x67fe, 0x4790, [0x94, 0xd5, 0xc4, 0x72, 0x6d, 0x7b, 0x6e, 0x6d]);
const PROFILE_HIGH: sys::GUID = guid(0xe7cbc309, 0x4f7a, 0x4b89, [0xaf, 0x2a, 0xd5, 0x37, 0xc9, 0x2b, 0xe3, 0x10]);

/// The nvEncodeAPI function table.
pub(crate) struct Nvenc {
    _lib: libloading::Library,
    f: sys::NV_ENCODE_API_FUNCTION_LIST,
}

// SAFETY: a table of driver function pointers; the library stays loaded for the struct's life.
unsafe impl Send for Nvenc {}
// SAFETY: as above.
unsafe impl Sync for Nvenc {}

static NVENC: OnceLock<std::result::Result<Arc<Nvenc>, String>> = OnceLock::new();

impl Nvenc {
    pub(crate) fn get() -> std::result::Result<Arc<Nvenc>, String> {
        NVENC.get_or_init(Self::load).clone()
    }

    fn load() -> std::result::Result<Arc<Nvenc>, String> {
        // SAFETY: loading the NVIDIA video encoding library; kept for the program's life.
        let lib = unsafe { libloading::Library::new("libnvidia-encode.so.1") }.map_err(|e| format!("no NVENC library (libnvidia-encode.so.1): {e}"))?;
        let max_version = sym!(lib, "NvEncodeAPIGetMaxSupportedVersion", MaxVersion);
        let create = sym!(lib, "NvEncodeAPICreateInstance", CreateInstance).ok_or("the NVENC library lacks NvEncodeAPICreateInstance")?;
        if let Some(mv) = max_version {
            let mut v = 0u32;
            // SAFETY: writes one u32.
            if unsafe { mv(&mut v) } == sys::NV_ENC_SUCCESS {
                let (major, minor) = (v >> 4, v & 0xF);
                if (major, minor) < (sys::NVENCAPI_MAJOR_VERSION, sys::NVENCAPI_MINOR_VERSION) {
                    return Err(format!(
                        "the NVIDIA driver supports NVENC API {major}.{minor}; FilmCraft needs {}.{} (update the driver)",
                        sys::NVENCAPI_MAJOR_VERSION,
                        sys::NVENCAPI_MINOR_VERSION
                    ));
                }
            }
        }
        let mut f = sys::NV_ENCODE_API_FUNCTION_LIST { version: FUNCTION_LIST_VER, ..Default::default() };
        // SAFETY: fills the versioned function table.
        let r = unsafe { create(&mut f) };
        if r != sys::NV_ENC_SUCCESS {
            return Err(format!("NvEncodeAPICreateInstance failed ({r})"));
        }
        Ok(Arc::new(Nvenc { _lib: lib, f }))
    }
}

fn enc_err(what: &str, status: sys::NVENCSTATUS) -> ExportError {
    ExportError::Encode(format!("NVENC {what} failed ({status})"))
}

fn status(what: &str, r: sys::NVENCSTATUS) -> Result<(), ExportError> {
    if r == sys::NV_ENC_SUCCESS { Ok(()) } else { Err(enc_err(what, r)) }
}

/// An open encode session with its buffers.
pub(crate) struct NvEncoder {
    api: Arc<Nvenc>,
    cuda: Arc<Cuda>,
    session: *mut c_void,
    w: u32,
    h: u32,
    rate: FrameRate,
    signal: filmcraft_export::ColorSignal,
    inputs: Vec<sys::NV_ENC_INPUT_PTR>,
    outputs: Vec<sys::NV_ENC_OUTPUT_PTR>,
    /// Buffer pairs handed to the encoder whose output has not been read yet, oldest first.
    queued: std::collections::VecDeque<usize>,
    /// Next buffer pair to use.
    next: usize,
    /// Packets returned so far (decoding order), for DTS.
    out_count: i64,
    bframes: u32,
    sps: Vec<u8>,
    pps: Vec<u8>,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    finished: bool,
}

// SAFETY: the session is used by one thread at a time (`&mut self`); NVENC allows a session to be
// driven from different threads sequentially with the CUDA context pushed.
unsafe impl Send for NvEncoder {}

impl NvEncoder {
    pub(crate) fn new(w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Result<Self, ExportError> {
        let api = Nvenc::get().map_err(ExportError::Encode)?;
        let cuda = Cuda::get().map_err(ExportError::Encode)?;
        let f = &api.f;
        let (Some(open), Some(preset_cfg), Some(init), Some(create_in), Some(create_out)) =
            (f.nvEncOpenEncodeSessionEx, f.nvEncGetEncodePresetConfigEx, f.nvEncInitializeEncoder, f.nvEncCreateInputBuffer, f.nvEncCreateBitstreamBuffer)
        else {
            return Err(ExportError::Encode("the NVENC function table is incomplete".into()));
        };
        let mut enc = Self {
            api: api.clone(),
            cuda: cuda.clone(),
            session: std::ptr::null_mut(),
            w,
            h,
            rate,
            signal: s.signal,
            inputs: Vec::new(),
            outputs: Vec::new(),
            queued: std::collections::VecDeque::new(),
            next: 0,
            out_count: 0,
            bframes: B_FRAMES,
            sps: Vec::new(),
            pps: Vec::new(),
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
            finished: false,
        };
        cuda.scope(|| -> Result<(), ExportError> {
            let mut params = sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS {
                version: OPEN_SESSION_VER,
                deviceType: sys::NV_ENC_DEVICE_TYPE_CUDA,
                device: cuda.ctx as *mut c_void,
                apiVersion: API_VERSION,
                ..Default::default()
            };
            let mut session: *mut c_void = std::ptr::null_mut();
            // SAFETY: valid versioned params and out-pointer; the CUDA context is current.
            status("nvEncOpenEncodeSessionEx", unsafe { open(&mut params, &mut session) })?;
            if session.is_null() {
                return Err(ExportError::Encode("NVENC opened no session".into()));
            }
            enc.session = session;

            // the P4 preset tuned for quality, then our rate control, GOP and colour on top
            let mut preset = sys::NV_ENC_PRESET_CONFIG { version: PRESET_CONFIG_VER, ..Default::default() };
            preset.presetCfg.version = CONFIG_VER;
            // SAFETY: valid session and versioned out-struct.
            status("nvEncGetEncodePresetConfigEx", unsafe { preset_cfg(session, CODEC_H264, PRESET_P4, sys::NV_ENC_TUNING_INFO_HIGH_QUALITY, &mut preset) })?;
            let mut cfg = preset.presetCfg;
            cfg.version = CONFIG_VER;
            cfg.profileGUID = match s.h264_profile {
                H264Profile::Baseline => PROFILE_BASELINE,
                H264Profile::Main => PROFILE_MAIN,
                H264Profile::High => PROFILE_HIGH,
            };
            let keyint = s.keyframe_distance.filter(|k| *k > 0).unwrap_or_else(|| (rate.num as f64 / rate.den as f64 * 2.0).round().max(1.0) as u32);
            cfg.gopLength = keyint;
            let bframes = if s.h264_profile == H264Profile::Baseline { 0 } else { B_FRAMES };
            enc.bframes = bframes;
            cfg.frameIntervalP = bframes as i32 + 1;
            cfg.frameFieldMode = sys::NV_ENC_PARAMS_FRAME_FIELD_MODE_FRAME;
            let kbps = s.bitrate_kbps.max(100);
            let max = s.max_bitrate_kbps.filter(|m| *m >= kbps).unwrap_or(kbps * 3 / 2);
            match s.bitrate_mode {
                BitrateMode::Cbr => {
                    cfg.rcParams.rateControlMode = sys::NV_ENC_PARAMS_RC_CBR;
                    cfg.rcParams.averageBitRate = kbps.saturating_mul(1000);
                    cfg.rcParams.maxBitRate = kbps.saturating_mul(1000);
                }
                _ => {
                    cfg.rcParams.rateControlMode = sys::NV_ENC_PARAMS_RC_VBR;
                    cfg.rcParams.averageBitRate = kbps.saturating_mul(1000);
                    cfg.rcParams.maxBitRate = max.saturating_mul(1000);
                }
            }
            cfg.rcParams.vbvBufferSize = 0;
            cfg.rcParams.vbvInitialDelay = 0;
            // the GPU's own two-pass analysis stands in for the software encoder's two passes
            cfg.rcParams.multiPass =
                if matches!(s.h264_pass, H264Pass::Single) { sys::NV_ENC_MULTI_PASS_DISABLED } else { sys::NV_ENC_TWO_PASS_FULL_RESOLUTION };
            cfg.rcParams.set_enableAQ(1);
            // SAFETY: the union field for the H.264 codec, which `encodeGUID` selects.
            let hc = unsafe { &mut cfg.encodeCodecConfig.h264Config };
            hc.idrPeriod = keyint;
            hc.set_repeatSPSPPS(0);
            hc.set_disableSPSPPS(0);
            hc.set_outputAUD(0);
            hc.chromaFormatIDC = 1;
            hc.useBFramesAsRef = sys::NV_ENC_BFRAME_REF_MODE_DISABLED;
            hc.entropyCodingMode =
                if s.h264_profile == H264Profile::Baseline { sys::NV_ENC_H264_ENTROPY_CODING_MODE_CAVLC } else { sys::NV_ENC_H264_ENTROPY_CODING_MODE_CABAC };
            hc.level = sys::NV_ENC_LEVEL_AUTOSELECT;
            hc.h264VUIParameters.videoSignalTypePresentFlag = 1;
            hc.h264VUIParameters.videoFormat = 5;
            hc.h264VUIParameters.videoFullRangeFlag = 0;
            hc.h264VUIParameters.colourDescriptionPresentFlag = 1;
            hc.h264VUIParameters.colourPrimaries = 1;
            hc.h264VUIParameters.transferCharacteristics = 1;
            hc.h264VUIParameters.colourMatrix = 1;

            let (dar_w, dar_h) = match s.pixel_aspect {
                Some((n, d)) if n > 0 && d > 0 => (w as u64 * n as u64, h as u64 * d as u64),
                _ => (w as u64, h as u64),
            };
            let g = gcd(dar_w, dar_h).max(1);
            let mut ip = sys::NV_ENC_INITIALIZE_PARAMS {
                version: INITIALIZE_PARAMS_VER,
                encodeGUID: CODEC_H264,
                presetGUID: PRESET_P4,
                encodeWidth: w,
                encodeHeight: h,
                darWidth: (dar_w / g).min(u32::MAX as u64) as u32,
                darHeight: (dar_h / g).min(u32::MAX as u64) as u32,
                frameRateNum: rate.num.max(1) as u32,
                frameRateDen: rate.den.max(1) as u32,
                enableEncodeAsync: 0,
                enablePTD: 1,
                encodeConfig: &mut cfg,
                maxEncodeWidth: w,
                maxEncodeHeight: h,
                tuningInfo: sys::NV_ENC_TUNING_INFO_HIGH_QUALITY,
                bufferFormat: sys::NV_ENC_BUFFER_FORMAT_NV12,
                ..Default::default()
            };
            // SAFETY: valid session; `ip` and the config it points at live across the call.
            status("nvEncInitializeEncoder", unsafe { init(session, &mut ip) })?;

            let n = BUFFERS + bframes as usize;
            for _ in 0..n {
                let mut cib = sys::NV_ENC_CREATE_INPUT_BUFFER {
                    version: CREATE_INPUT_BUFFER_VER,
                    width: w,
                    height: h,
                    bufferFmt: sys::NV_ENC_BUFFER_FORMAT_NV12,
                    ..Default::default()
                };
                // SAFETY: valid session and versioned struct.
                status("nvEncCreateInputBuffer", unsafe { create_in(session, &mut cib) })?;
                enc.inputs.push(cib.inputBuffer);
                let mut cbb = sys::NV_ENC_CREATE_BITSTREAM_BUFFER { version: CREATE_BITSTREAM_BUFFER_VER, ..Default::default() };
                // SAFETY: as above.
                status("nvEncCreateBitstreamBuffer", unsafe { create_out(session, &mut cbb) })?;
                enc.outputs.push(cbb.bitstreamBuffer);
            }
            Ok(())
        })
        .map_err(|e| ExportError::Encode(e.to_string()))??;
        Ok(enc)
    }

    /// Write `frame` as NV12 into input buffer `i`.
    fn fill_input(&mut self, i: usize, f: &EncoderFrame) -> Result<(), ExportError> {
        let (w, h) = (self.w as usize, self.h as usize);
        if f.width as usize != w || f.height as usize != h || f.rgba.len() < w * h * 4 {
            return Err(ExportError::Encode("frame size does not match the encoder".into()));
        }
        filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut self.y, &mut self.u, &mut self.v);
        let (Some(lock), Some(unlock)) = (self.api.f.nvEncLockInputBuffer, self.api.f.nvEncUnlockInputBuffer) else {
            return Err(ExportError::Encode("NVENC input buffers cannot be locked".into()));
        };
        let input = *self.inputs.get(i).ok_or_else(|| ExportError::Encode("no input buffer".into()))?;
        let mut lb = sys::NV_ENC_LOCK_INPUT_BUFFER { version: LOCK_INPUT_BUFFER_VER, inputBuffer: input, ..Default::default() };
        // SAFETY: a buffer this session created; locked until `unlock` below.
        status("nvEncLockInputBuffer", unsafe { lock(self.session, &mut lb) })?;
        let pitch = lb.pitch as usize;
        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let result = if lb.bufferDataPtr.is_null() || pitch < w {
            Err(ExportError::Encode("NVENC returned no input memory".into()))
        } else {
            let total = pitch * (h + ch);
            // SAFETY: the driver hands out `pitch × (h + h/2)` bytes of NV12 memory for a buffer
            // created with this size; we write within it row by row.
            let dst = unsafe { std::slice::from_raw_parts_mut(lb.bufferDataPtr as *mut u8, total) };
            for y in 0..h {
                dst[y * pitch..y * pitch + w].copy_from_slice(&self.y[y * w..(y + 1) * w]);
            }
            for y in 0..ch {
                let row = &mut dst[(h + y) * pitch..(h + y) * pitch + cw * 2];
                for x in 0..cw {
                    row[2 * x] = self.u[y * cw + x];
                    row[2 * x + 1] = self.v[y * cw + x];
                }
            }
            Ok(())
        };
        // SAFETY: unlocks what was locked above, exactly once.
        let r = unsafe { unlock(self.session, input) };
        result?;
        status("nvEncUnlockInputBuffer", r)
    }

    /// Read every finished output in `queued` (oldest first) into packets.
    fn drain(&mut self) -> Result<Vec<EncodedPacket>, ExportError> {
        let (Some(lock), Some(unlock)) = (self.api.f.nvEncLockBitstream, self.api.f.nvEncUnlockBitstream) else {
            return Err(ExportError::Encode("NVENC bitstreams cannot be locked".into()));
        };
        let mut packets = Vec::new();
        while let Some(i) = self.queued.pop_front() {
            let output = *self.outputs.get(i).ok_or_else(|| ExportError::Encode("no output buffer".into()))?;
            let mut lb = sys::NV_ENC_LOCK_BITSTREAM { version: LOCK_BITSTREAM_VER, outputBitstream: output, ..Default::default() };
            // SAFETY: a bitstream buffer this session created, whose picture is complete.
            status("nvEncLockBitstream", unsafe { lock(self.session, &mut lb) })?;
            let bytes = if lb.bitstreamBufferPtr.is_null() {
                Vec::new()
            } else {
                // SAFETY: the driver reports `bitstreamSizeInBytes` valid bytes at the pointer
                // until the buffer is unlocked.
                unsafe { std::slice::from_raw_parts(lb.bitstreamBufferPtr as *const u8, lb.bitstreamSizeInBytes as usize) }.to_vec()
            };
            let key = matches!(lb.pictureType, sys::NV_ENC_PIC_TYPE_IDR | sys::NV_ENC_PIC_TYPE_I);
            let pts = lb.outputTimeStamp as i64;
            // SAFETY: unlocks what was locked above.
            status("nvEncUnlockBitstream", unsafe { unlock(self.session, output) })?;
            let data = self.length_prefixed(&bytes);
            let delay = if self.bframes > 0 { 1 } else { 0 };
            let dts = (self.out_count - delay) * self.rate.den;
            self.out_count += 1;
            packets.push(EncodedPacket { data, key, duration: self.rate.den as u32, composition_offset: (pts - dts).clamp(0, i32::MAX as i64) as i32 });
        }
        Ok(packets)
    }

    /// Annex B access unit → length-prefixed NAL units; SPS / PPS are kept for the sample entry,
    /// access-unit delimiters dropped.
    fn length_prefixed(&mut self, au: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(au.len() + 16);
        for nal in annex_b_nals(au) {
            let Some(&head) = nal.first() else { continue };
            match head & 0x1f {
                7 => {
                    if self.sps.is_empty() {
                        self.sps = nal.to_vec();
                    }
                }
                8 => {
                    if self.pps.is_empty() {
                        self.pps = nal.to_vec();
                    }
                }
                9 => {}
                _ => {
                    out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    out.extend_from_slice(nal);
                }
            }
        }
        out
    }

    fn submit(&mut self, input: Option<(usize, u64)>) -> Result<Vec<EncodedPacket>, ExportError> {
        let Some(encode) = self.api.f.nvEncEncodePicture else {
            return Err(ExportError::Encode("NVENC cannot encode".into()));
        };
        let mut pp = sys::NV_ENC_PIC_PARAMS { version: PIC_PARAMS_VER, pictureStruct: sys::NV_ENC_PIC_STRUCT_FRAME, ..Default::default() };
        match input {
            Some((i, pts)) => {
                pp.inputWidth = self.w;
                pp.inputHeight = self.h;
                pp.inputBuffer = *self.inputs.get(i).ok_or_else(|| ExportError::Encode("no input buffer".into()))?;
                pp.outputBitstream = *self.outputs.get(i).ok_or_else(|| ExportError::Encode("no output buffer".into()))?;
                pp.bufferFmt = sys::NV_ENC_BUFFER_FORMAT_NV12;
                pp.inputTimeStamp = pts;
                pp.inputDuration = self.rate.den as u64;
                self.queued.push_back(i);
            }
            None => pp.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS,
        }
        let session = self.session;
        // SAFETY: valid session and picture parameters referring to this session's buffers.
        let r = self.cuda.scope(|| unsafe { encode(session, &mut pp) }).map_err(|e| ExportError::Encode(e.to_string()))?;
        match r {
            sys::NV_ENC_SUCCESS => {
                let cuda = self.cuda.clone();
                cuda.scope(|| self.drain()).map_err(|e| ExportError::Encode(e.to_string()))?
            }
            sys::NV_ENC_ERR_NEED_MORE_INPUT => Ok(Vec::new()),
            other => Err(enc_err("nvEncEncodePicture", other)),
        }
    }
}

impl Drop for NvEncoder {
    fn drop(&mut self) {
        if self.session.is_null() {
            return;
        }
        let f = &self.api.f;
        let _ = self.cuda.scope(|| {
            if let Some(d) = f.nvEncDestroyInputBuffer {
                for &b in &self.inputs {
                    // SAFETY: buffers this session created; destroyed once.
                    unsafe { d(self.session, b) };
                }
            }
            if let Some(d) = f.nvEncDestroyBitstreamBuffer {
                for &b in &self.outputs {
                    // SAFETY: as above.
                    unsafe { d(self.session, b) };
                }
            }
            if let Some(d) = f.nvEncDestroyEncoder {
                // SAFETY: the session this struct opened; destroyed once.
                unsafe { d(self.session) };
            }
        });
        self.session = std::ptr::null_mut();
    }
}

impl VideoEncoder for NvEncoder {
    fn sample_entry(&self) -> SampleEntry {
        let cfg = filmcraft_isobmff::AvcConfig::new(vec![self.sps.clone()], vec![self.pps.clone()], 4);
        let mut e = SampleEntry::avc(cfg, self.w as u16, self.h as u16);
        self.signal.apply_to(&mut e, false);
        e
    }
    fn timescale(&self) -> u32 {
        self.rate.num as u32
    }
    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>, ExportError> {
        if self.finished {
            return Err(ExportError::Encode("encoder already flushed".into()));
        }
        if self.queued.len() >= self.inputs.len() {
            return Err(ExportError::Encode("NVENC output queue overflow".into()));
        }
        let i = self.next;
        self.next = (self.next + 1) % self.inputs.len();
        self.fill_input(i, f)?;
        let pts = f.index.saturating_mul(self.rate.den as u64);
        self.submit(Some((i, pts)))
    }
    fn flush(&mut self) -> Result<Vec<EncodedPacket>, ExportError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.submit(None)
    }
    fn media_start(&self) -> Option<i64> {
        (self.bframes > 0).then_some(self.rate.den)
    }
    fn two_pass_internal(&self) -> bool {
        true
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// NAL units of an Annex B byte stream (3- or 4-byte start codes), without the start codes.
fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map_or(data.len(), |&n| n - 3);
        // a 4-byte start code leaves a zero before the next start code
        while e > s && data[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&data[s..e]);
        }
    }
    out
}

/// The export crate's factory: NVENC for 8-bit SDR H.264 exports when the GPU can take them.
pub(crate) fn nvenc_factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>, ExportError>> {
    if format != Format::H264 || s.format.is_mxf() || s.signal.is_hdr() || !hardware_encoding() {
        return None;
    }
    if w < 32 || h < 32 || w > 8192 || h > 8192 || !w.is_multiple_of(2) || !h.is_multiple_of(2) {
        return None;
    }
    match NvEncoder::new(w, h, rate, s) {
        Ok(e) => {
            log::info!("export: NVENC H.264 ({}x{})", w, h);
            Some(Ok(Box::new(e) as Box<dyn VideoEncoder>))
        }
        Err(e) => {
            log::info!("export: NVENC unavailable, using the software encoder: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annex_b_splitting() {
        let au = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9, 0];
        let nals = annex_b_nals(&au);
        assert_eq!(nals, vec![&[0x67u8, 1, 2][..], &[0x68, 3][..], &[0x65, 9, 9][..]]);
        assert!(annex_b_nals(&[]).is_empty());
        assert!(annex_b_nals(&[0, 0, 1]).is_empty());
    }
}
