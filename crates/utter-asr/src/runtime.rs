//! Loading ONNX Runtime, which `ort` loads at run time (`load-dynamic`), from Utter's own
//! runtime components (`utter-runtime`), each pinned and checksummed. On the CUDA backend:
//! onnxruntime's GPU build, the CUDA runtime with cuBLAS, and cuDNN. On the Vulkan backend
//! (AMD, Intel; docs/plan-vulkan.md): onnxruntime's CPU build and its WebGPU plugin provider,
//! registered here. Nothing is taken from other applications or from the system beyond the GPU
//! driver: verified by transcribing with a PATH holding only Windows' own folders.
//!
//! `UTTER_ORT_DYLIB` names a different onnxruntime library, for testing another build or for
//! platforms without pinned components yet (Linux, e.g. a GPU cluster): that build then finds its CUDA
//! libraries itself (`LD_LIBRARY_PATH`), and no component is required.

use std::path::PathBuf;
use std::sync::OnceLock;

use utter_runtime::Backend;

use crate::AsrError;

#[cfg(windows)]
const ORT_LIB: &str = "onnxruntime.dll";
#[cfg(not(windows))]
const ORT_LIB: &str = "libonnxruntime.so";

/// The components onnxruntime's CUDA provider loads its libraries from. cuFFT and cuRAND,
/// which the provider can also load, are not needed by these graphs: without them, 100 of 100
/// transcripts came out identical (2026-10-09).
pub const CUDA_COMPONENTS: &[&str] = &["cuda-runtime", "cudnn"];

/// The WebGPU plugin provider's component, its library, and the name sessions ask for it by.
pub const WEBGPU_COMPONENT: &str = "onnxruntime-ep-webgpu";
const WEBGPU_LIB: &str = "onnxruntime_providers_webgpu.dll";
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

fn init_inner() -> Result<PathBuf, AsrError> {
    let backend = utter_runtime::backend();
    let custom = LIBRARY.get().cloned().or_else(|| std::env::var_os("UTTER_ORT_DYLIB").map(PathBuf::from));
    let lib = match (&custom, backend) {
        (Some(path), _) => path.clone(),
        (None, Backend::Cuda) => component_dir("onnxruntime-gpu")?.join(ORT_LIB),
        (None, Backend::Vulkan) => component_dir("onnxruntime")?.join(ORT_LIB),
    };
    if !lib.is_file() {
        return Err(AsrError::Runtime(format!("{} does not exist", lib.display())));
    }
    // The plugin's folder also holds the shader compiler it loads (dxcompiler.dll, dxil.dll). A
    // custom onnxruntime build brings its own providers.
    let webgpu = match (backend, &custom) {
        (Backend::Vulkan, None) => Some(component_dir(WEBGPU_COMPONENT)?),
        _ => None,
    };
    if custom.is_none() {
        // Only the installed ones: a GPU session without them fails when it is created,
        // saying so.
        let dirs: Vec<PathBuf> = match &webgpu {
            Some(dir) => vec![dir.clone()],
            None => CUDA_COMPONENTS.iter().filter_map(|id| component_dir(id).ok()).collect(),
        };
        prepend_library_path(&dirs)?;
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
fn prepend_library_path(dirs: &[PathBuf]) -> Result<(), AsrError> {
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

/// Linux's loader reads LD_LIBRARY_PATH only at process start, so changing it here would do
/// nothing. Phase 4 (Linux) preloads these libraries itself; until then there are no Linux
/// components, and `component_dir` has already failed.
#[cfg(not(windows))]
fn prepend_library_path(_dirs: &[PathBuf]) -> Result<(), AsrError> {
    Ok(())
}
