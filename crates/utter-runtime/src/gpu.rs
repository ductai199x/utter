//! Keeps the GPU at full clocks while the user speaks.
//!
//! Under light load (a speech pass every few hundred milliseconds) an NVIDIA GPU settles in
//! P3, where the memory clock is half of P2's; the cleanup model's decoding is bound by memory
//! bandwidth, so the request after key up takes twice as long (docs/evals.md). Busy work
//! to hold P2 costs power and, from another process, steals time slices from the work that
//! matters. So the warmer runs one thread in a kernel that sleeps on the GPU (`nanosleep`, no
//! memory traffic) while the key is down, and stops at key up: the clocks stay up for a second
//! or so after, long enough for what follows, and nothing competes with it.
//!
//! It uses the CUDA driver API from the GPU driver itself (nvcuda.dll, libcuda.so.1), so it
//! needs no component. Without an NVIDIA GPU it does nothing.

use std::ffi::{CStr, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// Each launch naps this many times, 2 us each (the `nanosleep` in the PTX): about 2 ms.
/// `nanosleep` returns within twice what it asks for, so a launch ends within 4 ms, and
/// stopping takes at most that. (A deadline on `%globaltimer` instead hung a dictation: never
/// let a kernel depend on a clock to end.)
const NAPS: u32 = 1_000;

const PTX: &CStr = c".version 7.0
.target sm_70
.address_size 64

.visible .entry spin(.param .u32 naps)
{
    .reg .u32 %r<2>;
    .reg .pred %p;
    ld.param.u32 %r1, [naps];
LOOP:
    nanosleep.u32 2000;
    sub.u32 %r1, %r1, 1;
    setp.ne.u32 %p, %r1, 0;
    @%p bra LOOP;
    ret;
}
";

#[cfg(windows)]
const DRIVER: &str = "nvcuda.dll";
#[cfg(not(windows))]
const DRIVER: &str = "libcuda.so.1";

type CuResult = i32;
type CuDevice = i32;
type CuPtr = *mut c_void;

/// The driver entry points the warmer uses.
struct Driver {
    _lib: libloading::Library,
    ctx_set_current: unsafe extern "system" fn(CuPtr) -> CuResult,
    launch_kernel: unsafe extern "system" fn(
        CuPtr,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        CuPtr,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> CuResult,
    stream_synchronize: unsafe extern "system" fn(CuPtr) -> CuResult,
}

/// A loaded spin kernel on the primary context of GPU 0 (the one the speech model and
/// llama-server use).
struct Kernel {
    driver: Driver,
    ctx: CuPtr,
    function: CuPtr,
    stream: CuPtr,
}

// The handles are plain driver handles; the driver API is thread-safe once the context is
// current on the calling thread, which `spin` makes it.
unsafe impl Send for Kernel {}

impl Kernel {
    fn load() -> Result<Self, String> {
        // SAFETY: loading the GPU driver's own library and resolving documented entry points
        // with their documented signatures (cuda.h).
        unsafe {
            let lib = libloading::Library::new(DRIVER).map_err(|e| format!("{DRIVER}: {e}"))?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    *lib.get::<$ty>($name).map_err(|e| format!("{}: {e}", String::from_utf8_lossy($name)))?
                };
            }
            let init = sym!(b"cuInit\0", unsafe extern "system" fn(u32) -> CuResult);
            let device_get = sym!(b"cuDeviceGet\0", unsafe extern "system" fn(*mut CuDevice, i32) -> CuResult);
            let retain =
                sym!(b"cuDevicePrimaryCtxRetain\0", unsafe extern "system" fn(*mut CuPtr, CuDevice) -> CuResult);
            let ctx_set_current = sym!(b"cuCtxSetCurrent\0", unsafe extern "system" fn(CuPtr) -> CuResult);
            let module_load =
                sym!(b"cuModuleLoadData\0", unsafe extern "system" fn(*mut CuPtr, *const c_void) -> CuResult);
            let get_function = sym!(
                b"cuModuleGetFunction\0",
                unsafe extern "system" fn(*mut CuPtr, CuPtr, *const std::ffi::c_char) -> CuResult
            );
            let stream_create = sym!(b"cuStreamCreate\0", unsafe extern "system" fn(*mut CuPtr, u32) -> CuResult);
            let launch_kernel = sym!(
                b"cuLaunchKernel\0",
                unsafe extern "system" fn(
                    CuPtr,
                    u32,
                    u32,
                    u32,
                    u32,
                    u32,
                    u32,
                    u32,
                    CuPtr,
                    *mut *mut c_void,
                    *mut *mut c_void,
                ) -> CuResult
            );
            let stream_synchronize = sym!(b"cuStreamSynchronize\0", unsafe extern "system" fn(CuPtr) -> CuResult);

            let check = |what: &str, r: CuResult| if r == 0 { Ok(()) } else { Err(format!("{what} failed ({r})")) };
            check("cuInit", init(0))?;
            let mut device = 0;
            check("cuDeviceGet", device_get(&mut device, 0))?;
            let mut ctx = std::ptr::null_mut();
            check("cuDevicePrimaryCtxRetain", retain(&mut ctx, device))?;
            check("cuCtxSetCurrent", ctx_set_current(ctx))?;
            let mut module = std::ptr::null_mut();
            check("cuModuleLoadData", module_load(&mut module, PTX.as_ptr().cast()))?;
            let mut function = std::ptr::null_mut();
            check("cuModuleGetFunction", get_function(&mut function, module, c"spin".as_ptr()))?;
            let mut stream = std::ptr::null_mut();
            // CU_STREAM_NON_BLOCKING: never wait on (or hold up) the default stream.
            check("cuStreamCreate", stream_create(&mut stream, 1))?;
            Ok(Self {
                driver: Driver { _lib: lib, ctx_set_current, launch_kernel, stream_synchronize },
                ctx,
                function,
                stream,
            })
        }
    }

    /// Naps one GPU thread `NAPS` times, returning when it is done.
    fn spin(&self) -> Result<(), String> {
        let mut naps = NAPS;
        let mut params = [(&raw mut naps).cast::<c_void>()];
        // SAFETY: the kernel takes one u32 parameter, passed by pointer as cuLaunchKernel
        // requires; the stream and function belong to the current context.
        let r = unsafe {
            (self.driver.launch_kernel)(
                self.function,
                1,
                1,
                1,
                1,
                1,
                1,
                0,
                self.stream,
                params.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if r != 0 {
            return Err(format!("cuLaunchKernel failed ({r})"));
        }
        // SAFETY: a stream this kernel created.
        match unsafe { (self.driver.stream_synchronize)(self.stream) } {
            0 => Ok(()),
            r => Err(format!("cuStreamSynchronize failed ({r})")),
        }
    }
}

/// GPU 0 as the driver reports it: what first run checks before downloading anything.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub memory_bytes: u64,
    /// Free GPU memory right now (what other apps, a game say, leave), when the driver says.
    pub free_bytes: Option<u64>,
    /// The newest CUDA version the driver supports, as major * 1000 + minor * 10 (13040: 13.4).
    pub driver_cuda: i32,
}

/// The oldest driver CUDA version the components run on: they are built against CUDA 13, and
/// any 13.x driver runs 13.x runtimes (minor version compatibility).
pub const MIN_DRIVER_CUDA: i32 = 13_000;

/// Asks the GPU driver about GPU 0; an error says why there is no usable NVIDIA GPU.
pub fn probe() -> Result<GpuInfo, String> {
    // SAFETY: the GPU driver's own library, documented entry points with their documented
    // signatures (cuda.h); buffers are sized as passed.
    unsafe {
        let lib = libloading::Library::new(DRIVER).map_err(|_| "no NVIDIA GPU driver found".to_owned())?;
        macro_rules! sym {
            ($name:literal, $ty:ty) => {
                *lib.get::<$ty>($name).map_err(|e| format!("{}: {e}", String::from_utf8_lossy($name)))?
            };
        }
        let init = sym!(b"cuInit\0", unsafe extern "system" fn(u32) -> CuResult);
        let version = sym!(b"cuDriverGetVersion\0", unsafe extern "system" fn(*mut i32) -> CuResult);
        let device_get = sym!(b"cuDeviceGet\0", unsafe extern "system" fn(*mut CuDevice, i32) -> CuResult);
        let get_name =
            sym!(b"cuDeviceGetName\0", unsafe extern "system" fn(*mut std::ffi::c_char, i32, CuDevice) -> CuResult);
        let total_mem = sym!(b"cuDeviceTotalMem_v2\0", unsafe extern "system" fn(*mut usize, CuDevice) -> CuResult);
        let check = |what: &str, r: CuResult| if r == 0 { Ok(()) } else { Err(format!("{what} failed ({r})")) };
        check("cuInit (is an NVIDIA GPU installed?)", init(0))?;
        let mut driver_cuda = 0;
        check("cuDriverGetVersion", version(&mut driver_cuda))?;
        let mut device = 0;
        check("cuDeviceGet", device_get(&mut device, 0))?;
        let mut name = [0 as std::ffi::c_char; 256];
        check("cuDeviceGetName", get_name(name.as_mut_ptr(), name.len() as i32, device))?;
        let mut memory = 0usize;
        check("cuDeviceTotalMem", total_mem(&mut memory, device))?;
        // Free memory needs a context: the primary one, released at once.
        let free_bytes = (|| {
            let retain = *lib
                .get::<unsafe extern "system" fn(*mut CuPtr, CuDevice) -> CuResult>(b"cuDevicePrimaryCtxRetain\0")
                .ok()?;
            let release =
                *lib.get::<unsafe extern "system" fn(CuDevice) -> CuResult>(b"cuDevicePrimaryCtxRelease_v2\0").ok()?;
            let set_current = *lib.get::<unsafe extern "system" fn(CuPtr) -> CuResult>(b"cuCtxSetCurrent\0").ok()?;
            let mem_info =
                *lib.get::<unsafe extern "system" fn(*mut usize, *mut usize) -> CuResult>(b"cuMemGetInfo_v2\0").ok()?;
            let mut ctx = std::ptr::null_mut();
            if retain(&mut ctx, device) != 0 {
                return None;
            }
            let (mut free, mut total) = (0usize, 0usize);
            let ok = set_current(ctx) == 0 && mem_info(&mut free, &mut total) == 0;
            set_current(std::ptr::null_mut());
            release(device);
            ok.then_some(free as u64)
        })();
        Ok(GpuInfo {
            name: CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned(),
            memory_bytes: memory as u64,
            free_bytes,
            driver_cuda,
        })
    }
}

/// Holds the GPU's clocks up between `start` and `stop`. Cheap to call either repeatedly.
pub struct GpuWarmer {
    on: Arc<AtomicBool>,
    wake: std::sync::mpsc::Sender<()>,
    _thread: Option<JoinHandle<()>>,
}

impl GpuWarmer {
    /// Loads the spin kernel on its own thread. Without an NVIDIA GPU (or with a driver too
    /// old for the kernel) it logs why once and `start` does nothing.
    pub fn new() -> Self {
        let on = Arc::new(AtomicBool::new(false));
        let (wake, woken) = std::sync::mpsc::channel::<()>();
        let flag = on.clone();
        let thread = std::thread::Builder::new()
            .name("utter-gpu-warm".into())
            .spawn(move || {
                if std::env::var_os("UTTER_NO_GPU_WARM").is_some() {
                    tracing::info!("GPU warmer off: UTTER_NO_GPU_WARM is set");
                    return;
                }
                let kernel = match Kernel::load() {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::info!("GPU warmer off: {e}");
                        return;
                    }
                };
                // SAFETY: the context was retained by `load` and stays alive with the process.
                if unsafe { (kernel.driver.ctx_set_current)(kernel.ctx) } != 0 {
                    tracing::info!("GPU warmer off: cuCtxSetCurrent failed");
                    return;
                }
                while woken.recv().is_ok() {
                    while flag.load(Ordering::Relaxed) {
                        if let Err(e) = kernel.spin() {
                            tracing::warn!("GPU warmer stopped: {e}");
                            return;
                        }
                    }
                }
            })
            .ok();
        Self { on, wake, _thread: thread }
    }

    /// Starts holding the clocks up (key down).
    pub fn start(&self) {
        if !self.on.swap(true, Ordering::Relaxed) {
            let _ = self.wake.send(());
        }
    }

    /// Stops within 4 ms (key up).
    pub fn stop(&self) {
        self.on.store(false, Ordering::Relaxed);
    }
}

impl Default for GpuWarmer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GpuWarmer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Every spin ends close to its length (needs an NVIDIA GPU: `cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn spins_end_on_time() {
        let kernel = Kernel::load().expect("an NVIDIA GPU");
        let mut worst = Duration::ZERO;
        for _ in 0..3000 {
            let t = std::time::Instant::now();
            kernel.spin().unwrap();
            worst = worst.max(t.elapsed());
        }
        eprintln!("worst spin {worst:?}");
        assert!(worst < Duration::from_millis(12), "worst spin {worst:?}");
    }
}
