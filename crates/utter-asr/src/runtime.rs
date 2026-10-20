//! Loading ONNX Runtime, which `ort` loads at run time (`load-dynamic`), from Utter's own
//! runtime components (`utter-runtime`), each pinned and checksummed:
//! - CUDA: onnxruntime's GPU build, the CUDA runtime with cuBLAS, and cuDNN;
//! - Vulkan and Metal (docs/platforms.md, docs/platforms.md): onnxruntime's CPU
//!   build and its WebGPU plugin provider (Dawn over D3D12 or Vulkan), registered here; on macOS
//!   the build has WebGPU (Dawn over Metal) built in, and registering the plugin as well failed
//!   ("EP library is already registered", macOS 26.7);
//! - CPU: onnxruntime's CPU build alone.
//!
//! Nothing is taken from other applications or from the system beyond the GPU driver: verified
//! on Windows by transcribing with a PATH holding only Windows' own folders.
//!
//! `UTTER_ORT_DYLIB` names a different onnxruntime library, for testing another build: that
//! build then finds its own GPU libraries, and no component is required.

use std::path::PathBuf;
use std::sync::OnceLock;

use utter_runtime::Backend;

use crate::AsrError;

#[cfg(windows)]
const ORT_LIB: &str = "onnxruntime.dll";
#[cfg(target_os = "macos")]
const ORT_LIB: &str = "libonnxruntime.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const ORT_LIB: &str = "libonnxruntime.so";

/// The components onnxruntime's CUDA provider loads its libraries from. cuFFT and cuRAND,
/// which the provider can also load, are not needed by these graphs: without them, 100 of 100
/// transcripts came out identical (2026-10-09). On Windows the provider loads them lazily; on
/// Linux its library names libcurand.so.10 as a dependency, so it does not load without it
/// (WSL, 2026-10-18).
#[cfg(not(target_os = "linux"))]
pub const CUDA_COMPONENTS: &[&str] = &["cuda-runtime", "cudnn"];
#[cfg(target_os = "linux")]
pub const CUDA_COMPONENTS: &[&str] = &["cuda-runtime", "cudnn", "curand"];

/// The WebGPU plugin provider's component, its library, and the name sessions ask for it by.
pub const WEBGPU_COMPONENT: &str = "onnxruntime-ep-webgpu";
#[cfg(windows)]
const WEBGPU_LIB: &str = "onnxruntime_providers_webgpu.dll";
#[cfg(target_os = "macos")]
const WEBGPU_LIB: &str = "libonnxruntime_providers_webgpu.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const WEBGPU_LIB: &str = "libonnxruntime_providers_webgpu.so";
pub const WEBGPU_EP: &str = "WebGpuExecutionProvider";

static INIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static LIBRARY: OnceLock<PathBuf> = OnceLock::new();

/// Loads onnxruntime from `lib` instead of Utter's components, as `UTTER_ORT_DYLIB` does:
/// Android's, shipped in the app's native library folder. Call it before anything loads
/// onnxruntime; later calls do nothing.
pub fn use_library(lib: PathBuf) {
    let _ = LIBRARY.set(lib);
}

/// Loads ONNX Runtime once per process and returns the library's path.
pub fn init() -> Result<PathBuf, AsrError> {
    INIT.get_or_init(|| init_inner().map_err(|e| e.to_string())).clone().map_err(AsrError::Runtime)
}

/// The onnxruntime build a backend runs.
pub fn ort_component(backend: Backend) -> &'static str {
    match backend {
        Backend::Cuda => "onnxruntime-gpu",
        Backend::Vulkan | Backend::Metal | Backend::Cpu => "onnxruntime",
    }
}

fn init_inner() -> Result<PathBuf, AsrError> {
    let backend = utter_runtime::backend();
    let custom = LIBRARY.get().cloned().or_else(|| std::env::var_os("UTTER_ORT_DYLIB").map(PathBuf::from));
    let lib = match &custom {
        Some(path) => path.clone(),
        None => component_dir(ort_component(backend))?.join(ORT_LIB),
    };
    if !lib.is_file() {
        return Err(AsrError::Runtime(format!("{} does not exist", lib.display())));
    }
    // On Windows the plugin's folder also holds the shader compiler it loads (dxcompiler.dll,
    // dxil.dll). A custom onnxruntime build brings its own providers.
    let webgpu = match (backend.uses_webgpu() && !cfg!(target_os = "macos"), &custom) {
        (true, None) => Some(component_dir(WEBGPU_COMPONENT)?),
        _ => None,
    };
    if custom.is_none() {
        // Only the installed ones: a GPU session without them fails when it is created,
        // saying so.
        let dirs: Vec<PathBuf> = match (&webgpu, backend) {
            (Some(dir), _) => vec![dir.clone()],
            (None, Backend::Cuda) => CUDA_COMPONENTS.iter().filter_map(|id| component_dir(id).ok()).collect(),
            (None, _) => Vec::new(),
        };
        make_findable(&dirs)?;
    }
    let builder = ort::init_from(&lib).map_err(|e| AsrError::Runtime(format!("{}: {e}", lib.display())))?;
    builder.with_name("utter").commit();
    if let Some(dir) = webgpu {
        let plugin = dir.join(WEBGPU_LIB);
        // Registered for the life of the process; the returned handle only unregisters it.
        ort::environment::Environment::current()?
            .register_ep_library(WEBGPU_EP, &plugin)
            .map_err(|e| AsrError::Runtime(format!("{}: {e}", plugin.display())))?;
    }
    tracing::info!(lib = %lib.display(), ?backend, "onnxruntime loaded");
    Ok(lib)
}

fn component_dir(id: &str) -> Result<PathBuf, AsrError> {
    let c = utter_runtime::component(id).map_err(|e| AsrError::Runtime(e.to_string()))?;
    c.installed_dir().map_err(|e| AsrError::Runtime(e.to_string()))
}

/// Windows searches PATH, as it is at the moment of each load, for a DLL's dependencies.
#[cfg(windows)]
fn make_findable(dirs: &[PathBuf]) -> Result<(), AsrError> {
    let var = "PATH";
    let mut paths: Vec<PathBuf> = dirs.to_vec();
    if let Some(existing) = std::env::var_os(var) {
        paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(paths).map_err(|e| AsrError::Runtime(format!("{var}: {e}")))?;
    // SAFETY: called once, from `init`, before any session (and so any thread of ONNX
    // Runtime's) exists; nothing else in the process reads the environment concurrently.
    unsafe { std::env::set_var(var, joined) };
    Ok(())
}

/// Linux's and macOS's loaders read their search variables only at process start, but reuse a
/// library already loaded under the same name: so every shared library in `dirs` is loaded now,
/// by full path, for good (the CUDA runtime, cuBLAS, cuDNN and the libraries cuDNN opens by
/// name later). They depend on one another; loading repeats until nothing more loads, so no
/// order has to be kept by hand.
#[cfg(unix)]
fn make_findable(dirs: &[PathBuf]) -> Result<(), AsrError> {
    use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_NOW};

    let mut pending: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.contains(".so") || name.ends_with(".dylib")
        })
        .collect();
    pending.sort();
    loop {
        let before = pending.len();
        let mut failed = Vec::new();
        for path in pending {
            // SAFETY: loading Utter's own pinned, checksummed libraries; their initializers
            // have no preconditions. Kept loaded for the life of the process.
            match unsafe { Library::open(Some(&path), RTLD_NOW | RTLD_GLOBAL) } {
                Ok(lib) => std::mem::forget(lib),
                Err(e) => failed.push((path, e.to_string())),
            }
        }
        if failed.is_empty() {
            return Ok(());
        }
        if failed.len() == before {
            // Nothing more loads: the sessions will say what is missing; log what did not.
            for (path, e) in &failed {
                tracing::warn!(lib = %path.display(), "not preloaded: {e}");
            }
            return Ok(());
        }
        pending = failed.into_iter().map(|(p, _)| p).collect();
    }
}

/// Linux: ends the process at exit before onnxruntime tears itself down. The ort crate releases
/// its environment from the executable's .fini_array, which runs after the C++ destructors of
/// libraries opened at run time, onnxruntime and its CUDA provider among them: the release then
/// freed memory a second time and glibc aborted ("corrupted double-linked list") at every exit
/// after a CUDA session (WSL, RTX 3090 Ti, 2026-10-18; Windows and macOS release it earlier).
/// on_exit handlers run before the destructors registered earlier, so this one, registered once
/// a session exists, flushes C's buffers and leaves with the exit status; the OS and the driver
/// take back the rest.
#[cfg(target_os = "linux")]
pub(crate) fn skip_teardown_at_exit() {
    use std::ffi::{c_int, c_void};
    unsafe extern "C" {
        fn on_exit(function: extern "C" fn(c_int, *mut c_void), arg: *mut c_void) -> c_int;
        fn fflush(stream: *mut c_void) -> c_int;
        fn _exit(status: c_int) -> !;
    }
    extern "C" fn leave(status: c_int, _: *mut c_void) {
        // SAFETY: flushing every C stream, then ending the process; called by exit() only.
        unsafe {
            fflush(std::ptr::null_mut());
            _exit(status)
        }
    }
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    // SAFETY: registers a handler with glibc; it lives as long as the process.
    REGISTERED.call_once(|| unsafe {
        on_exit(leave, std::ptr::null_mut());
    });
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn skip_teardown_at_exit() {}
