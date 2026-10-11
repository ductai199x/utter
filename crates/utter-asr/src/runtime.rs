//! Loading ONNX Runtime, which `ort` loads at run time (`load-dynamic`), from Utter's own
//! runtime components (`utter-runtime`): onnxruntime's GPU build, the CUDA runtime with
//! cuBLAS, and cuDNN, each pinned and checksummed. Nothing is taken from other applications
//! or from the system beyond the GPU driver: verified by transcribing with a PATH holding only
//! Windows' own folders.
//!
//! `UTTER_ORT_DYLIB` names a different onnxruntime library, for testing another build or for
//! platforms without pinned components yet (Linux, e.g. a GPU cluster): that build then finds its CUDA
//! libraries itself (`LD_LIBRARY_PATH`), and no component is required.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::AsrError;

#[cfg(windows)]
const ORT_LIB: &str = "onnxruntime.dll";
#[cfg(not(windows))]
const ORT_LIB: &str = "libonnxruntime.so";

/// The components onnxruntime's CUDA provider loads its libraries from. cuFFT and cuRAND,
/// which the provider can also load, are not needed by these graphs: without them, 100 of 100
/// transcripts came out identical (2026-10-09).
pub const CUDA_COMPONENTS: &[&str] = &["cuda-runtime", "cudnn"];

static INIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// Loads ONNX Runtime once per process and returns the library's path.
pub fn init() -> Result<PathBuf, AsrError> {
    INIT.get_or_init(|| init_inner().map_err(|e| e.to_string())).clone().map_err(AsrError::Runtime)
}

fn init_inner() -> Result<PathBuf, AsrError> {
    let custom = std::env::var_os("UTTER_ORT_DYLIB").map(PathBuf::from);
    let lib = match &custom {
        Some(path) => path.clone(),
        None => component_dir("onnxruntime-gpu")?.join(ORT_LIB),
    };
    if !lib.is_file() {
        return Err(AsrError::Runtime(format!("{} does not exist", lib.display())));
    }
    if custom.is_none() {
        let mut dirs = Vec::new();
        for id in CUDA_COMPONENTS {
            dirs.push(component_dir(id)?);
        }
        prepend_library_path(&dirs)?;
    }
    let builder = ort::init_from(&lib).map_err(|e| AsrError::Runtime(format!("{}: {e}", lib.display())))?;
    builder.with_name("utter").commit();
    tracing::info!(lib = %lib.display(), "onnxruntime loaded");
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
