//! NVENC hardware H.264 and HEVC encoding for exports (Linux).
//!
//! Rules as for the decoder: every `unsafe` block has a `// SAFETY:` comment; every failure is a
//! `Result`. The encoder is created by [`nvenc_factory`], which the export crate asks first; when
//! the GPU, the driver or the settings rule it out, the factory answers `None` and FilmCraft's own
//! H.264 encoder takes over, so an export never fails for want of hardware.
//!
//! Frames arrive as sRGB RGBA8 (or, for HDR, encoded R'G'B' floats) and are converted to
//! limited-range NV12 / P010 on the CPU with the same arithmetic as the software path, so hardware
//! and software exports match in colour. NVENC returns Annex B access units in decoding order;
//! the parameter sets go into the sample entry's `avcC` / `hvcC`, the rest becomes
//! length-prefixed MP4 samples.
//!
//! H.264 is 8-bit SDR (HDR H.264 stays with the software encoder, which writes it at 8 bits as
//! well). HEVC is Main for SDR, or Main 10 when the settings ask for it and always for HDR
//! (PQ / HLG, with the mastering display and light level SEI for PQ).

use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use filmcraft_export::{
    BitrateMode, EncodePerformance, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format, H264Pass, H264Profile, HevcProfile, VideoEncoder,
};
use filmcraft_isobmff::SampleEntry;
use filmcraft_time::FrameRate;

use super::cuda::{Cuda, sym};
use super::sys;

/// B-frames between references; 2 is what the software encoder's defaults produce.
const B_FRAMES: u32 = 2;
/// Input / output buffers in flight: the B-frame delay plus headroom.
const BUFFERS: usize = 8;

/// Whether exports may use the GPU's encoders: Settings ▸ Media ▸ Enable hardware accelerated
/// encoding (`filmcraft_export::set_hardware_encoding`), unless `FILMCRAFT_NO_NVENC` is set in
/// the environment.
pub fn hardware_encoding() -> bool {
    filmcraft_export::hardware_encoding() && std::env::var_os("FILMCRAFT_NO_NVENC").is_none()
}

pub fn set_hardware_encoding(on: bool) {
    filmcraft_export::set_hardware_encoding(on);
}

/// The codec of an encode session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Codec {
    H264,
    Hevc,
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
const CODEC_HEVC: sys::GUID = guid(0x790cdc88, 0x4522, 0x4d7b, [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03]);
const PROFILE_HEVC_MAIN: sys::GUID = guid(0xb514c39a, 0xb55b, 0x40fa, [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec]);
const PROFILE_HEVC_MAIN10: sys::GUID = guid(0xfa4d2b6c, 0x3a5b, 0x411a, [0x80, 0x18, 0x0a, 0x3f, 0x5e, 0x3c, 0x9b, 0xe5]);
const CAPS_PARAM_VER: u32 = struct_version(1);

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
    codec: Codec,
    /// P010 input, 10-bit output (HEVC Main 10).
    ten_bit: bool,
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
    vps: Vec<u8>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    y10: Vec<u16>,
    u10: Vec<u16>,
    v10: Vec<u16>,
    /// HDR10 static metadata sent with every IDR picture (HEVC PQ).
    hdr10: Option<(sys::MASTERING_DISPLAY_INFO, sys::CONTENT_LIGHT_LEVEL)>,
    finished: bool,
}

// SAFETY: the session is used by one thread at a time (`&mut self`); NVENC allows a session to be
// driven from different threads sequentially with the CUDA context pushed.
unsafe impl Send for NvEncoder {}

impl NvEncoder {
    pub(crate) fn new(codec: Codec, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Result<Self, ExportError> {
        let api = Nvenc::get().map_err(ExportError::Encode)?;
        let cuda = Cuda::get().map_err(ExportError::Encode)?;
        let f = &api.f;
        let (Some(open), Some(preset_cfg), Some(init), Some(create_in), Some(create_out)) =
            (f.nvEncOpenEncodeSessionEx, f.nvEncGetEncodePresetConfigEx, f.nvEncInitializeEncoder, f.nvEncCreateInputBuffer, f.nvEncCreateBitstreamBuffer)
        else {
            return Err(ExportError::Encode("the NVENC function table is incomplete".into()));
        };
        let ten_bit = codec == Codec::Hevc && (s.hevc_profile == HevcProfile::Main10 || s.signal.is_hdr());
        let (codec_guid, buffer_format) = match codec {
            Codec::H264 => (CODEC_H264, sys::NV_ENC_BUFFER_FORMAT_NV12),
            Codec::Hevc => (CODEC_HEVC, if ten_bit { sys::NV_ENC_BUFFER_FORMAT_YUV420_10BIT } else { sys::NV_ENC_BUFFER_FORMAT_NV12 }),
        };
        let mut enc = Self {
            api: api.clone(),
            cuda: cuda.clone(),
            session: std::ptr::null_mut(),
            codec,
            ten_bit,
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
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
            y10: Vec::new(),
            u10: Vec::new(),
            v10: Vec::new(),
            hdr10: (codec == Codec::Hevc && s.signal.transfer == 16).then(hdr10_bt2020),
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
            status("nvEncGetEncodePresetConfigEx", unsafe { preset_cfg(session, codec_guid, PRESET_P4, sys::NV_ENC_TUNING_INFO_HIGH_QUALITY, &mut preset) })?;
            let mut cfg = preset.presetCfg;
            cfg.version = CONFIG_VER;
            cfg.profileGUID = match (codec, s.h264_profile) {
                (Codec::Hevc, _) if ten_bit => PROFILE_HEVC_MAIN10,
                (Codec::Hevc, _) => PROFILE_HEVC_MAIN,
                (Codec::H264, H264Profile::Baseline) => PROFILE_BASELINE,
                (Codec::H264, H264Profile::Main) => PROFILE_MAIN,
                (Codec::H264, H264Profile::High) => PROFILE_HIGH,
            };
            let keyint = s.keyframe_distance.filter(|k| *k > 0).unwrap_or_else(|| (rate.num as f64 / rate.den as f64 * 2.0).round().max(1.0) as u32);
            cfg.gopLength = keyint;
            let bframes = if codec == Codec::H264 && s.h264_profile == H264Profile::Baseline { 0 } else { B_FRAMES };
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
            let two_pass = match codec {
                Codec::H264 => !matches!(s.h264_pass, H264Pass::Single),
                Codec::Hevc => s.bitrate_mode == BitrateMode::Vbr2Pass,
            };
            cfg.rcParams.multiPass = if two_pass { sys::NV_ENC_TWO_PASS_FULL_RESOLUTION } else { sys::NV_ENC_MULTI_PASS_DISABLED };
            cfg.rcParams.set_enableAQ(1);
            let signal = s.signal;
            let set_vui = |vui: &mut sys::NV_ENC_CONFIG_H264_VUI_PARAMETERS| {
                vui.videoSignalTypePresentFlag = 1;
                vui.videoFormat = 5;
                vui.videoFullRangeFlag = 0;
                vui.colourDescriptionPresentFlag = 1;
                vui.colourPrimaries = signal.primaries as _;
                vui.transferCharacteristics = signal.transfer as _;
                vui.colourMatrix = signal.matrix as _;
            };
            match codec {
                Codec::H264 => {
                    // SAFETY: the union field for the H.264 codec, which `encodeGUID` selects.
                    let hc = unsafe { &mut cfg.encodeCodecConfig.h264Config };
                    hc.idrPeriod = keyint;
                    hc.set_repeatSPSPPS(0);
                    hc.set_disableSPSPPS(0);
                    hc.set_outputAUD(0);
                    hc.chromaFormatIDC = 1;
                    hc.useBFramesAsRef = sys::NV_ENC_BFRAME_REF_MODE_DISABLED;
                    hc.entropyCodingMode = if s.h264_profile == H264Profile::Baseline {
                        sys::NV_ENC_H264_ENTROPY_CODING_MODE_CAVLC
                    } else {
                        sys::NV_ENC_H264_ENTROPY_CODING_MODE_CABAC
                    };
                    hc.level = sys::NV_ENC_LEVEL_AUTOSELECT;
                    set_vui(&mut hc.h264VUIParameters);
                }
                Codec::Hevc => {
                    // SAFETY: the union field for the HEVC codec, which `encodeGUID` selects.
                    let hc = unsafe { &mut cfg.encodeCodecConfig.hevcConfig };
                    hc.idrPeriod = keyint;
                    hc.set_repeatSPSPPS(0);
                    hc.set_disableSPSPPS(0);
                    hc.set_outputAUD(0);
                    hc.set_chromaFormatIDC(1);
                    let depth = if ten_bit { sys::NV_ENC_BIT_DEPTH_10 } else { sys::NV_ENC_BIT_DEPTH_8 };
                    hc.inputBitDepth = depth;
                    hc.outputBitDepth = depth;
                    hc.useBFramesAsRef = sys::NV_ENC_BFRAME_REF_MODE_DISABLED;
                    hc.level = sys::NV_ENC_LEVEL_AUTOSELECT;
                    hc.tier = sys::NV_ENC_TIER_HEVC_MAIN;
                    if enc.hdr10.is_some() {
                        hc.set_outputMasteringDisplay(1);
                        hc.set_outputMaxCll(1);
                    }
                    set_vui(&mut hc.hevcVUIParameters);
                }
            }

            let (dar_w, dar_h) = match s.pixel_aspect {
                Some((n, d)) if n > 0 && d > 0 => (w as u64 * n as u64, h as u64 * d as u64),
                _ => (w as u64, h as u64),
            };
            let g = gcd(dar_w, dar_h).max(1);
            let mut ip = sys::NV_ENC_INITIALIZE_PARAMS {
                version: INITIALIZE_PARAMS_VER,
                encodeGUID: codec_guid,
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
                bufferFormat: buffer_format,
                ..Default::default()
            };
            // SAFETY: valid session; `ip` and the config it points at live across the call.
            status("nvEncInitializeEncoder", unsafe { init(session, &mut ip) })?;

            let n = BUFFERS + bframes as usize;
            for _ in 0..n {
                let mut cib =
                    sys::NV_ENC_CREATE_INPUT_BUFFER { version: CREATE_INPUT_BUFFER_VER, width: w, height: h, bufferFmt: buffer_format, ..Default::default() };
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
        if self.ten_bit {
            let (kr, kb) = self.signal.kr_kb();
            let hdr = f.hdr.filter(|_| self.signal.is_hdr());
            filmcraft_export::to_yuv420_10(f.rgba, hdr, w, h, kr, kb, &mut self.y10, &mut self.u10, &mut self.v10);
        } else {
            filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut self.y, &mut self.u, &mut self.v);
        }
        let (Some(lock), Some(unlock)) = (self.api.f.nvEncLockInputBuffer, self.api.f.nvEncUnlockInputBuffer) else {
            return Err(ExportError::Encode("NVENC input buffers cannot be locked".into()));
        };
        let input = *self.inputs.get(i).ok_or_else(|| ExportError::Encode("no input buffer".into()))?;
        let mut lb = sys::NV_ENC_LOCK_INPUT_BUFFER { version: LOCK_INPUT_BUFFER_VER, inputBuffer: input, ..Default::default() };
        // SAFETY: a buffer this session created; locked until `unlock` below.
        status("nvEncLockInputBuffer", unsafe { lock(self.session, &mut lb) })?;
        let pitch = lb.pitch as usize;
        let ch = h.div_ceil(2);
        let bps = if self.ten_bit { 2 } else { 1 };
        let result = if lb.bufferDataPtr.is_null() || pitch < w * bps {
            Err(ExportError::Encode("NVENC returned no input memory".into()))
        } else {
            let total = pitch * (h + ch);
            // SAFETY: the driver hands out `pitch × (h + h/2)` bytes of NV12 / P010 memory for a
            // buffer created with this size; we write within it row by row.
            let dst = unsafe { std::slice::from_raw_parts_mut(lb.bufferDataPtr as *mut u8, total) };
            if self.ten_bit { write_p010(dst, pitch, w, h, &self.y10, &self.u10, &self.v10) } else { write_nv12(dst, pitch, w, h, &self.y, &self.u, &self.v) }
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
            let kind = match (self.codec, head) {
                (Codec::H264, h) => match h & 0x1f {
                    7 => ParamSet::Sps,
                    8 => ParamSet::Pps,
                    9 => ParamSet::Delimiter,
                    _ => ParamSet::None,
                },
                (Codec::Hevc, h) => match (h >> 1) & 0x3f {
                    32 => ParamSet::Vps,
                    33 => ParamSet::Sps,
                    34 => ParamSet::Pps,
                    35 => ParamSet::Delimiter,
                    _ => ParamSet::None,
                },
            };
            let keep = |v: &mut Vec<u8>| {
                if v.is_empty() {
                    *v = nal.to_vec();
                }
            };
            match kind {
                ParamSet::Vps => keep(&mut self.vps),
                ParamSet::Sps => keep(&mut self.sps),
                ParamSet::Pps => keep(&mut self.pps),
                ParamSet::Delimiter => {}
                ParamSet::None => {
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
                pp.bufferFmt = if self.ten_bit { sys::NV_ENC_BUFFER_FORMAT_YUV420_10BIT } else { sys::NV_ENC_BUFFER_FORMAT_NV12 };
                pp.inputTimeStamp = pts;
                if let Some((md, cll)) = self.hdr10.as_mut() {
                    // SAFETY: the union field for HEVC (only HEVC sessions carry `hdr10`); the
                    // pointers stay valid across the encode call below (fields of `self`).
                    let hp = unsafe { &mut pp.codecPicParams.hevcPicParams };
                    hp.pMasteringDisplay = md;
                    hp.pMaxCll = cll;
                }
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
        let mut e = match self.codec {
            Codec::H264 => SampleEntry::avc(filmcraft_isobmff::AvcConfig::new(vec![self.sps.clone()], vec![self.pps.clone()], 4), self.w as u16, self.h as u16),
            Codec::Hevc => SampleEntry::hevc(hvcc(&self.vps, &self.sps, &self.pps, self.ten_bit, self.rate), self.w as u16, self.h as u16),
        };
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

/// The export crate's factory: NVENC for 8-bit SDR H.264 and for HEVC exports when the GPU can
/// take them. H.264 falls back to FilmCraft's own encoder; HEVC has none, so its failure is the
/// export's error.
pub(crate) fn nvenc_factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>, ExportError>> {
    let codec = match format {
        Format::H264 if !s.format.is_mxf() && !s.signal.is_hdr() => Codec::H264,
        Format::Hevc if !s.format.is_mxf() => Codec::Hevc,
        _ => return None,
    };
    if !hardware_encoding() || s.performance == EncodePerformance::Software {
        return None;
    }
    let max = if codec == Codec::Hevc { 8192 } else { 4096 };
    if w < 64 || h < 64 || w > max || h > max || !w.is_multiple_of(2) || !h.is_multiple_of(2) {
        return None;
    }
    if codec == Codec::Hevc {
        let c = caps();
        if !c.hevc {
            return Some(Err(ExportError::Unsupported("this GPU has no hardware HEVC encoder".into())));
        }
        if (s.hevc_profile == HevcProfile::Main10 || s.signal.is_hdr()) && !c.hevc_10bit {
            return Some(Err(ExportError::Unsupported("this GPU cannot encode 10-bit HEVC (Main 10, needed for HDR)".into())));
        }
    }
    match NvEncoder::new(codec, w, h, rate, s) {
        Ok(e) => {
            log::info!("export: NVENC {codec:?}{} ({}x{})", if e.ten_bit { " 10-bit" } else { "" }, w, h);
            Some(Ok(Box::new(e) as Box<dyn VideoEncoder>))
        }
        Err(e) if codec == Codec::Hevc => Some(Err(ExportError::Encode(format!("NVENC HEVC: {e}")))),
        Err(e) => {
            log::info!("export: NVENC unavailable, using the software encoder: {e}");
            None
        }
    }
}

/// What the GPU's encoder can do, asked once per process (opening a session takes a moment).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Caps {
    pub(crate) hevc: bool,
    pub(crate) hevc_10bit: bool,
}

static CAPS: Mutex<Option<Caps>> = Mutex::new(None);

pub(crate) fn caps() -> Caps {
    let mut g = CAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = *g {
        return c;
    }
    let c = query_caps().unwrap_or_else(|e| {
        log::info!("NVENC: {e}");
        Caps::default()
    });
    *g = Some(c);
    c
}

fn query_caps() -> Result<Caps, String> {
    let api = Nvenc::get()?;
    let cuda = Cuda::get()?;
    let f = &api.f;
    let (Some(open), Some(count), Some(guids), Some(get_caps), Some(destroy)) =
        (f.nvEncOpenEncodeSessionEx, f.nvEncGetEncodeGUIDCount, f.nvEncGetEncodeGUIDs, f.nvEncGetEncodeCaps, f.nvEncDestroyEncoder)
    else {
        return Err("the NVENC function table is incomplete".into());
    };
    cuda.scope(|| -> Result<Caps, String> {
        let mut params = sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS {
            version: OPEN_SESSION_VER,
            deviceType: sys::NV_ENC_DEVICE_TYPE_CUDA,
            device: cuda.ctx as *mut c_void,
            apiVersion: API_VERSION,
            ..Default::default()
        };
        let mut session: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid versioned params and out-pointer; the CUDA context is current.
        let r = unsafe { open(&mut params, &mut session) };
        if r != sys::NV_ENC_SUCCESS || session.is_null() {
            return Err(format!("no encode session ({r})"));
        }
        let mut n = 0u32;
        // SAFETY: valid session; writes one u32.
        let mut list = if unsafe { count(session, &mut n) } == sys::NV_ENC_SUCCESS { vec![guid(0, 0, 0, [0; 8]); n.min(64) as usize] } else { Vec::new() };
        let mut got = 0u32;
        // SAFETY: `list` holds `list.len()` GUIDs, the capacity passed.
        if !list.is_empty() && unsafe { guids(session, list.as_mut_ptr(), list.len() as u32, &mut got) } == sys::NV_ENC_SUCCESS {
            list.truncate(got as usize);
        } else {
            list.clear();
        }
        let same = |a: &sys::GUID, b: &sys::GUID| a.Data1 == b.Data1 && a.Data2 == b.Data2 && a.Data3 == b.Data3 && a.Data4 == b.Data4;
        let hevc = list.iter().any(|g| same(g, &CODEC_HEVC));
        let hevc_10bit = hevc && {
            let mut q = sys::NV_ENC_CAPS_PARAM { version: CAPS_PARAM_VER, capsToQuery: sys::NV_ENC_CAPS_SUPPORT_10BIT_ENCODE, ..Default::default() };
            let mut v = 0i32;
            // SAFETY: valid session, versioned caps query and out-pointer.
            let r = unsafe { get_caps(session, CODEC_HEVC, &mut q, &mut v) };
            r == sys::NV_ENC_SUCCESS && v != 0
        };
        // SAFETY: the session opened above; destroyed once.
        unsafe { destroy(session) };
        Ok(Caps { hevc, hevc_10bit })
    })
    .map_err(|e| e.to_string())?
}

/// The export crate's availability probe for hardware-only formats.
pub(crate) fn nvenc_probe(format: Format) -> bool {
    hardware_encoding() && format == Format::Hevc && caps().hevc
}

/// Kinds of NAL unit the encoder sorts out of its access units.
enum ParamSet {
    Vps,
    Sps,
    Pps,
    Delimiter,
    None,
}

/// BT.2020 / D65 mastering display, 1000 / 0.0001 cd/m², MaxCLL / MaxFALL unknown (0): what the
/// sample entry's `mdcv` / `clli` say ([`filmcraft_export::ColorSignal::apply_to`]).
fn hdr10_bt2020() -> (sys::MASTERING_DISPLAY_INFO, sys::CONTENT_LIGHT_LEVEL) {
    // chromaticities in 0.00002 units, luminance in 0.0001 cd/m²
    let p = |x: f64, y: f64| sys::CHROMA_POINTS { x: (x / 0.00002).round() as u16, y: (y / 0.00002).round() as u16 };
    let md = sys::MASTERING_DISPLAY_INFO {
        g: p(0.170, 0.797),
        b: p(0.131, 0.046),
        r: p(0.708, 0.292),
        whitePoint: p(0.3127, 0.3290),
        maxLuma: 10_000_000,
        minLuma: 1,
    };
    (md, sys::CONTENT_LIGHT_LEVEL { maxContentLightLevel: 0, maxPicAverageLightLevel: 0 })
}

/// NV12: the Y plane, then interleaved Cb Cr, rows `pitch` bytes apart.
fn write_nv12(dst: &mut [u8], pitch: usize, w: usize, h: usize, y: &[u8], u: &[u8], v: &[u8]) -> Result<(), ExportError> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let short = || ExportError::Encode("NVENC input buffer too small".into());
    for r in 0..h {
        let d = dst.get_mut(r * pitch..r * pitch + w).ok_or_else(short)?;
        d.copy_from_slice(y.get(r * w..(r + 1) * w).ok_or_else(short)?);
    }
    for r in 0..ch {
        let row = dst.get_mut((h + r) * pitch..(h + r) * pitch + cw * 2).ok_or_else(short)?;
        let (us, vs) = (u.get(r * cw..(r + 1) * cw).ok_or_else(short)?, v.get(r * cw..(r + 1) * cw).ok_or_else(short)?);
        for (pair, (&a, &b)) in row.as_chunks_mut::<2>().0.iter_mut().zip(us.iter().zip(vs)) {
            *pair = [a, b];
        }
    }
    Ok(())
}

/// P010: as NV12 with 16-bit little-endian samples holding the 10-bit value in the top bits.
fn write_p010(dst: &mut [u8], pitch: usize, w: usize, h: usize, y: &[u16], u: &[u16], v: &[u16]) -> Result<(), ExportError> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let short = || ExportError::Encode("NVENC input buffer too small".into());
    for r in 0..h {
        let d = dst.get_mut(r * pitch..r * pitch + w * 2).ok_or_else(short)?;
        for (o, &s) in d.as_chunks_mut::<2>().0.iter_mut().zip(y.get(r * w..(r + 1) * w).ok_or_else(short)?) {
            *o = (s << 6).to_le_bytes();
        }
    }
    for r in 0..ch {
        let row = dst.get_mut((h + r) * pitch..(h + r) * pitch + cw * 4).ok_or_else(short)?;
        let (us, vs) = (u.get(r * cw..(r + 1) * cw).ok_or_else(short)?, v.get(r * cw..(r + 1) * cw).ok_or_else(short)?);
        for (quad, (&a, &b)) in row.as_chunks_mut::<4>().0.iter_mut().zip(us.iter().zip(vs)) {
            let ([a0, a1], [b0, b1]) = ((a << 6).to_le_bytes(), (b << 6).to_le_bytes());
            *quad = [a0, a1, b0, b1];
        }
    }
    Ok(())
}

/// The `hvcC` record for the stream's parameter sets. Profile, tier, level and their flags are
/// copied from the SPS's `profile_tier_level` (byte-aligned at the start of the SPS payload).
fn hvcc(vps: &[u8], sps: &[u8], pps: &[u8], ten_bit: bool, rate: FrameRate) -> filmcraft_isobmff::HevcConfig {
    use filmcraft_isobmff::{HevcConfig, HevcNalArray};
    let rbsp = unescape(sps);
    let b = |i: usize| rbsp.get(i).copied().unwrap_or(0);
    // [0..2] NAL header; [2] vps id (4), max_sub_layers_minus1 (3), temporal_id_nesting (1);
    // [3] profile space (2), tier (1), profile idc (5); [4..8] compatibility flags;
    // [8..14] 48 constraint flags; [14] level idc
    let sub_layers = (b(2) >> 1) & 7;
    let compat = u32::from_be_bytes([b(4), b(5), b(6), b(7)]);
    let constraints = (8..14).fold(0u64, |acc, i| (acc << 8) | b(i) as u64);
    let depth = if ten_bit { 10 } else { 8 };
    let fps256 = (rate.num.max(0) as f64 * 256.0 / rate.den.max(1) as f64).round().clamp(0.0, 65535.0) as u16;
    let array = |nal_type: u8, nal: &[u8]| HevcNalArray { completeness: true, nal_type, nalus: if nal.is_empty() { Vec::new() } else { vec![nal.to_vec()] } };
    HevcConfig {
        general_profile_space: b(3) >> 6,
        general_tier_flag: b(3) & 0x20 != 0,
        general_profile_idc: b(3) & 0x1f,
        general_profile_compatibility_flags: compat,
        general_constraint_indicator_flags: constraints,
        general_level_idc: b(14),
        min_spatial_segmentation_idc: 0,
        parallelism_type: 0,
        chroma_format_idc: 1,
        bit_depth_luma: depth,
        bit_depth_chroma: depth,
        avg_frame_rate: fps256,
        constant_frame_rate: 0,
        num_temporal_layers: sub_layers + 1,
        temporal_id_nested: b(2) & 1 != 0,
        length_size: 4,
        arrays: vec![array(32, vps), array(33, sps), array(34, pps)],
    }
}

/// A NAL unit's payload without emulation-prevention bytes (`00 00 03` → `00 00`).
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &x in nal {
        if zeros >= 2 && x == 3 {
            zeros = 0;
            continue;
        }
        zeros = if x == 0 { zeros + 1 } else { 0 };
        out.push(x);
    }
    out
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

    /// A moving picture with gradients and edges, straight RGBA8.
    fn picture(w: usize, h: usize, i: usize) -> Vec<u8> {
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let box_ = (x + 3 * i) % w < w / 4 && y > h / 3 && y < 2 * h / 3;
                px[o] = if box_ { 230 } else { (x * 255 / w) as u8 };
                px[o + 1] = ((y * 255 / h) as u8).wrapping_add(i as u8);
                px[o + 2] = if box_ { 40 } else { ((x + y) * 255 / (w + h)) as u8 };
                px[o + 3] = 255;
            }
        }
        px
    }

    fn settings(format: Format, profile: HevcProfile, signal: filmcraft_export::ColorSignal) -> ExportSettings {
        ExportSettings { format, hevc_profile: profile, signal, bitrate_kbps: 8_000, keyframe_distance: Some(12), ..Default::default() }
    }

    /// Encode `n` frames, decode them with FilmCraft's own decoder; the luma of every decoded
    /// frame (display order) and of the source, at the stream's bit depth.
    fn round_trip(codec: Codec, s: &ExportSettings, w: usize, h: usize, n: usize) -> (SampleEntry, Vec<(Vec<u16>, Vec<u16>)>) {
        let rate = FrameRate::new(30, 1);
        let mut enc = NvEncoder::new(codec, w as u32, h as u32, rate, s).unwrap();
        let mut packets = Vec::new();
        let mut sources = Vec::new();
        for i in 0..n {
            let rgba = picture(w, h, i);
            // HDR input: the same picture as encoded R'G'B' floats
            let hdr: Vec<f32> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();
            let (kr, kb) = s.signal.kr_kb();
            let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
            if enc.ten_bit {
                let f = s.signal.is_hdr().then_some(hdr.as_slice());
                filmcraft_export::to_yuv420_10(&rgba, f, w, h, kr, kb, &mut y, &mut u, &mut v);
                sources.push(y);
            } else {
                let (mut y8, mut u8_, mut v8) = (Vec::new(), Vec::new(), Vec::new());
                filmcraft_export::rgba_to_yuv420_8(&rgba, w, h, &mut y8, &mut u8_, &mut v8);
                sources.push(y8.iter().map(|&b| b as u16).collect());
            }
            let f = EncoderFrame { width: w as u32, height: h as u32, rgba: &rgba, hdr: s.signal.is_hdr().then_some(hdr.as_slice()), index: i as u64 };
            packets.extend(enc.encode(&f).unwrap());
        }
        packets.extend(enc.flush().unwrap());
        assert_eq!(packets.len(), n, "one packet per frame");
        assert!(packets[0].key, "starts with a key frame");
        let entry = enc.sample_entry();
        let mut dec = filmcraft_codecs::software_video_decoder(&entry).unwrap();
        let delay = enc.media_start().unwrap_or(0);
        let mut frames = Vec::new();
        for (i, p) in packets.iter().enumerate() {
            let pts = i as i64 * rate.den - delay + p.composition_offset as i64;
            frames.extend(dec.decode(&p.data, pts).unwrap());
        }
        frames.extend(dec.flush());
        frames.sort_by_key(|f| f.pts);
        assert_eq!(frames.len(), n, "every frame decodes");
        let lumas = frames
            .iter()
            .map(|f| match &f.frame.data {
                filmcraft_frame::PixelData::Yuv8 { planes, .. } => planes[0].iter().map(|&b| b as u16).collect(),
                filmcraft_frame::PixelData::Yuv16 { planes, .. } => planes[0].to_vec(),
                other => panic!("unexpected picture {other:?}"),
            })
            .collect::<Vec<Vec<u16>>>();
        (entry, lumas.into_iter().zip(sources).collect())
    }

    fn psnr(a: &[u16], b: &[u16], max: f64) -> f64 {
        let mse = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum::<f64>() / a.len().max(1) as f64;
        if mse == 0.0 { 99.0 } else { 10.0 * (max * max / mse).log10() }
    }

    fn hevc_on_this_machine() -> bool {
        let ok = Cuda::get().is_ok() && caps().hevc;
        if !ok {
            eprintln!("SKIPPED: no NVENC HEVC encoder");
        }
        ok
    }

    #[test]
    fn hevc_main_round_trips_through_our_decoder() {
        if !hevc_on_this_machine() {
            return;
        }
        let s = settings(Format::Hevc, HevcProfile::Main, Default::default());
        let (entry, frames) = round_trip(Codec::Hevc, &s, 256, 144, 30);
        let filmcraft_isobmff::CodecConfig::Hevc(c) = &entry.codec else { panic!("not hvcC") };
        assert_eq!((c.general_profile_idc, c.bit_depth_luma, c.chroma_format_idc, c.length_size), (1, 8, 1, 4));
        assert!(c.general_level_idc > 0);
        assert_eq!(c.arrays.iter().map(|a| (a.nal_type, a.nalus.len())).collect::<Vec<_>>(), [(32, 1), (33, 1), (34, 1)]);
        for (i, (got, want)) in frames.iter().enumerate() {
            let p = psnr(got, want, 255.0);
            assert!(p > 36.0, "frame {i}: {p:.1} dB");
        }
    }

    #[test]
    fn hevc_main10_and_pq_hdr_are_ten_bit_with_hdr_signalling() {
        if !hevc_on_this_machine() || !caps().hevc_10bit {
            return;
        }
        let s = settings(Format::Hevc, HevcProfile::Main10, Default::default());
        let (entry, frames) = round_trip(Codec::Hevc, &s, 256, 144, 20);
        let filmcraft_isobmff::CodecConfig::Hevc(c) = &entry.codec else { panic!("not hvcC") };
        assert_eq!((c.general_profile_idc, c.bit_depth_luma, c.bit_depth_chroma), (2, 10, 10));
        for (i, (got, want)) in frames.iter().enumerate() {
            let p = psnr(got, want, 1023.0);
            assert!(p > 36.0, "Main 10 frame {i}: {p:.1} dB");
        }
        // an HDR (PQ) sequence: Main 10 whatever the profile setting, BT.2020 / PQ signalled
        let s = settings(Format::Hevc, HevcProfile::Main, filmcraft_export::ColorSignal::PQ);
        let (entry, frames) = round_trip(Codec::Hevc, &s, 256, 144, 12);
        let filmcraft_isobmff::CodecConfig::Hevc(c) = &entry.codec else { panic!("not hvcC") };
        assert_eq!((c.general_profile_idc, c.bit_depth_luma), (2, 10));
        let v = entry.video.as_ref().unwrap();
        assert!(matches!(v.color, Some(filmcraft_isobmff::ColorInfo::Nclx { primaries: 9, transfer: 16, matrix: 9, .. })), "{:?}", v.color);
        assert!(v.mastering_display.is_some() && v.content_light.is_some());
        for (i, (got, want)) in frames.iter().enumerate() {
            let p = psnr(got, want, 1023.0);
            assert!(p > 36.0, "PQ frame {i}: {p:.1} dB");
        }
    }

    #[test]
    fn the_hevc_profile_tier_level_is_read_from_the_sps() {
        // NAL header 42 01; vps 0, 1 sub-layer, nesting; Main, Main tier; compat 0x60000000;
        // constraints 90 00 00 00 00 00 (with an emulation-prevention byte); level 93 (3.1)
        let sps = [0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x5d, 0xa0];
        let c = hvcc(&[0x40, 0x01], &sps, &[0x44, 0x01], false, FrameRate::new(30000, 1001));
        assert_eq!((c.general_profile_space, c.general_tier_flag, c.general_profile_idc), (0, false, 1));
        assert_eq!(c.general_profile_compatibility_flags, 0x6000_0000);
        assert_eq!(c.general_constraint_indicator_flags, 0x9000_0000_0000);
        assert_eq!(c.general_level_idc, 93);
        assert_eq!((c.num_temporal_layers, c.temporal_id_nested), (1, true));
        assert_eq!(c.avg_frame_rate, 7672);
        // a truncated SPS gives zeros, never a panic
        let c = hvcc(&[], &[0x42], &[], true, FrameRate::new(0, 0));
        assert_eq!((c.general_profile_idc, c.general_level_idc, c.bit_depth_luma), (0, 0, 10));
    }
}
