//! The CUDA driver API, loaded at run time from `libcuda.so.1` (the NVIDIA driver). Nothing is
//! linked: a system without the driver gets [`Cuda::get`] = `Err`, and FilmCraft's own codecs
//! carry on.
//!
//! One process-wide [`Cuda`] holds the library and the primary context of device 0. Every call
//! into NVDEC / NVENC happens with that context pushed on the calling thread ([`Cuda::scope`]);
//! the driver API is thread-safe with a context current per thread.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::{Arc, OnceLock};

use super::sys;

/// A driver-API failure: the error's name (`CUDA_ERROR_INVALID_VALUE`) and where it happened.
#[derive(Clone, Debug)]
pub(crate) struct CudaError {
    pub(crate) what: &'static str,
    pub(crate) code: sys::CUresult,
    pub(crate) name: String,
}

impl std::fmt::Display for CudaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} ({})", self.what, self.name, self.code)
    }
}

impl std::error::Error for CudaError {}

/// Load a function pointer of type `T` by name; `None` when the library lacks it.
macro_rules! sym {
    ($lib:expr, $name:literal, $t:ty) => {{
        // SAFETY: the symbol is declared with the type the driver exports it as (the `t*`
        // typedefs of nv-codec-headers); a missing symbol is `None`, never a wrong call.
        let f: Option<libloading::Symbol<'_, $t>> = unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.ok();
        f.map(|s| *s)
    }};
}
pub(crate) use sym;

/// The functions FilmCraft uses. `Option` function pointers as bindgen generates them; a
/// `None` means the driver does not export the symbol and the loader refused the library.
pub(crate) struct Cuda {
    _lib: libloading::Library,
    pub(crate) ctx: sys::CUcontext,
    pub(crate) device_name: String,
    ctx_push: sys::tcuCtxPushCurrent_v2,
    ctx_pop: sys::tcuCtxPopCurrent_v2,
    memcpy_2d: sys::tcuMemcpy2D_v2,
    get_error_name: sys::tcuGetErrorName,
}

// SAFETY: the context handle is a driver-owned token that may be pushed from any thread (the
// driver API is thread-safe); the library stays loaded for the lifetime of the struct.
unsafe impl Send for Cuda {}
// SAFETY: as above; the handles are never mutated after construction.
unsafe impl Sync for Cuda {}

static CUDA: OnceLock<Result<Arc<Cuda>, String>> = OnceLock::new();

impl Cuda {
    /// The process-wide driver handle, loaded on first use. `Err` names why there is none.
    pub(crate) fn get() -> Result<Arc<Cuda>, String> {
        CUDA.get_or_init(Self::load).clone()
    }

    fn load() -> Result<Arc<Cuda>, String> {
        // SAFETY: loading the NVIDIA driver library runs its initialisers, which the driver
        // documents as safe to run in any process; the handle is kept for the program's life.
        let lib = unsafe { libloading::Library::new("libcuda.so.1") }.map_err(|e| format!("no NVIDIA driver (libcuda.so.1): {e}"))?;
        let init = sym!(lib, "cuInit", sys::tcuInit).flatten();
        let device_get = sym!(lib, "cuDeviceGet", sys::tcuDeviceGet).flatten();
        let device_get_count = sym!(lib, "cuDeviceGetCount", sys::tcuDeviceGetCount).flatten();
        let device_get_name = sym!(lib, "cuDeviceGetName", sys::tcuDeviceGetName).flatten();
        let primary_retain = sym!(lib, "cuDevicePrimaryCtxRetain", sys::tcuDevicePrimaryCtxRetain).flatten();
        let ctx_push = sym!(lib, "cuCtxPushCurrent_v2", sys::tcuCtxPushCurrent_v2).flatten();
        let ctx_pop = sym!(lib, "cuCtxPopCurrent_v2", sys::tcuCtxPopCurrent_v2).flatten();
        let memcpy_2d = sym!(lib, "cuMemcpy2D_v2", sys::tcuMemcpy2D_v2).flatten();
        let get_error_name = sym!(lib, "cuGetErrorName", sys::tcuGetErrorName).flatten();
        let (Some(init), Some(device_get), Some(device_get_count), Some(primary_retain), Some(ctx_push), Some(ctx_pop), Some(memcpy_2d)) =
            (init, device_get, device_get_count, primary_retain, ctx_push, ctx_pop, memcpy_2d)
        else {
            return Err("the NVIDIA driver library lacks the CUDA driver API".into());
        };
        let name_of = |code: sys::CUresult| -> String {
            let mut p: *const c_char = std::ptr::null();
            // SAFETY: `cuGetErrorName` writes a pointer to a static string owned by the driver.
            match get_error_name.map(|f| unsafe { f(code, &mut p) }) {
                Some(sys::CUDA_SUCCESS) if !p.is_null() => {
                    // SAFETY: the driver returned a valid NUL-terminated static string.
                    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
                }
                _ => format!("CUDA error {code}"),
            }
        };
        let check = |what: &'static str, code: sys::CUresult| -> Result<(), String> {
            if code == sys::CUDA_SUCCESS { Ok(()) } else { Err(format!("{what}: {}", name_of(code))) }
        };
        // SAFETY: plain driver calls with valid out-pointers to locals.
        unsafe {
            check("cuInit", init(0))?;
            let mut count: c_int = 0;
            check("cuDeviceGetCount", device_get_count(&mut count))?;
            if count < 1 {
                return Err("no CUDA-capable NVIDIA GPU".into());
            }
            let mut device: sys::CUdevice = 0;
            check("cuDeviceGet", device_get(&mut device, 0))?;
            let mut name = [0 as c_char; 128];
            let device_name = match device_get_name.map(|f| f(name.as_mut_ptr(), name.len() as c_int, device)) {
                Some(sys::CUDA_SUCCESS) => CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned(),
                _ => "NVIDIA GPU".to_string(),
            };
            let mut ctx: sys::CUcontext = std::ptr::null_mut();
            check("cuDevicePrimaryCtxRetain", primary_retain(&mut ctx, device))?;
            if ctx.is_null() {
                return Err("cuDevicePrimaryCtxRetain returned no context".into());
            }
            log::info!("NVIDIA hardware codecs: {device_name}");
            let _ = device;
            Ok(Arc::new(Cuda { _lib: lib, ctx, device_name, ctx_push: Some(ctx_push), ctx_pop: Some(ctx_pop), memcpy_2d: Some(memcpy_2d), get_error_name }))
        }
    }

    pub(crate) fn error(&self, what: &'static str, code: sys::CUresult) -> CudaError {
        let mut p: *const c_char = std::ptr::null();
        // SAFETY: as in `load`: the driver writes a pointer to one of its static strings.
        let name = match self.get_error_name.map(|f| unsafe { f(code, &mut p) }) {
            // SAFETY: a valid NUL-terminated static string.
            Some(sys::CUDA_SUCCESS) if !p.is_null() => unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned(),
            _ => format!("CUDA error {code}"),
        };
        CudaError { what, code, name }
    }

    pub(crate) fn check(&self, what: &'static str, code: sys::CUresult) -> Result<(), CudaError> {
        if code == sys::CUDA_SUCCESS { Ok(()) } else { Err(self.error(what, code)) }
    }

    /// Run `f` with the device context current on this thread.
    pub(crate) fn scope<T>(&self, f: impl FnOnce() -> T) -> Result<T, CudaError> {
        let (Some(push), Some(pop)) = (self.ctx_push, self.ctx_pop) else {
            return Err(CudaError { what: "cuCtxPushCurrent", code: sys::CUDA_ERROR_INVALID_VALUE, name: "not loaded".into() });
        };
        // SAFETY: the primary context handle is valid for the process; push/pop are balanced.
        self.check("cuCtxPushCurrent", unsafe { push(self.ctx) })?;
        let out = f();
        let mut popped: sys::CUcontext = std::ptr::null_mut();
        // SAFETY: pops the context this thread pushed above.
        let r = unsafe { pop(&mut popped) };
        self.check("cuCtxPopCurrent", r)?;
        Ok(out)
    }

    /// Copy `rows` rows of `width_bytes` from device memory (`src` + `src_pitch`) into `dst`
    /// (tightly packed). The context must be current.
    pub(crate) fn copy_rows_to_host(&self, src: u64, src_pitch: usize, width_bytes: usize, rows: usize, dst: &mut [u8]) -> Result<(), CudaError> {
        if dst.len() < width_bytes * rows {
            return Err(CudaError { what: "cuMemcpy2D", code: sys::CUDA_ERROR_INVALID_VALUE, name: "host buffer too small".into() });
        }
        let Some(memcpy) = self.memcpy_2d else {
            return Err(CudaError { what: "cuMemcpy2D", code: sys::CUDA_ERROR_INVALID_VALUE, name: "not loaded".into() });
        };
        let copy = sys::CUDA_MEMCPY2D {
            srcXInBytes: 0,
            srcY: 0,
            srcMemoryType: sys::CU_MEMORYTYPE_DEVICE,
            srcHost: std::ptr::null(),
            srcDevice: src,
            srcArray: std::ptr::null_mut(),
            srcPitch: src_pitch,
            dstXInBytes: 0,
            dstY: 0,
            dstMemoryType: sys::CU_MEMORYTYPE_HOST,
            dstHost: dst.as_mut_ptr() as *mut c_void,
            dstDevice: 0,
            dstArray: std::ptr::null_mut(),
            dstPitch: width_bytes,
            WidthInBytes: width_bytes,
            Height: rows,
        };
        // SAFETY: source is a mapped decoder surface the caller owns for the duration of the call;
        // destination is `dst`, checked above to hold `rows × width_bytes`.
        self.check("cuMemcpy2D", unsafe { memcpy(&copy) })
    }
}
