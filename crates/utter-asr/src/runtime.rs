//! Loading ONNX Runtime, which `ort` loads at run time (`load-dynamic`).
//!
//! Where the library comes from, in order:
//! 1. `UTTER_ORT_DYLIB`: the full path of `onnxruntime.dll` / `libonnxruntime.so`;
//! 2. in development, `tools/.venv`'s onnxruntime-gpu, found by walking up from the current
//!    directory and from the executable.
//!
//! The CUDA provider needs the CUDA and cuDNN libraries. In development they come from the
//! nvidia-* wheels in the same environment: their folders are put at the front of the
//! process's library search path before the first session is created. The installer will
//! ship these libraries beside the executable instead.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::AsrError;

#[cfg(windows)]
const ORT_LIB: &str = "onnxruntime.dll";
#[cfg(not(windows))]
const ORT_LIB: &str = "libonnxruntime.so";

#[cfg(windows)]
const SITE_PACKAGES: &[&str] = &["Lib", "site-packages"];
#[cfg(not(windows))]
const SITE_PACKAGES: &[&str] = &["lib", "python3.12", "site-packages"];

#[cfg(windows)]
const LIB_EXT: &str = "dll";
#[cfg(not(windows))]
const LIB_EXT: &str = "so";

static INIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// Loads ONNX Runtime once per process and returns the library's path.
pub fn init() -> Result<PathBuf, AsrError> {
    INIT.get_or_init(|| init_inner().map_err(|e| e.to_string())).clone().map_err(AsrError::Runtime)
}

fn init_inner() -> Result<PathBuf, AsrError> {
    let lib = locate()?;
    if let Some(site) = site_packages_of(&lib) {
        prepend_library_path(&nvidia_lib_dirs(&site))?;
    }
    let builder = ort::init_from(&lib).map_err(|e| AsrError::Runtime(format!("{}: {e}", lib.display())))?;
    builder.with_name("utter").commit();
    tracing::info!(lib = %lib.display(), "onnxruntime loaded");
    Ok(lib)
}

fn locate() -> Result<PathBuf, AsrError> {
    if let Some(path) = std::env::var_os("UTTER_ORT_DYLIB") {
        let path = PathBuf::from(path);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(AsrError::Runtime(format!("UTTER_ORT_DYLIB={} does not exist", path.display())))
        };
    }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok()];
    for start in starts.into_iter().flatten() {
        for dir in start.ancestors() {
            let mut candidate = dir.join("tools").join(".venv");
            for part in SITE_PACKAGES {
                candidate.push(part);
            }
            candidate.push("onnxruntime");
            candidate.push("capi");
            candidate.push(ORT_LIB);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(AsrError::Runtime(format!(
        "{ORT_LIB} not found: set UTTER_ORT_DYLIB, or run `uv sync` in tools/ (docs/local-setup.md)"
    )))
}

/// `.../site-packages` for a library at `.../site-packages/onnxruntime/capi/<lib>`.
fn site_packages_of(lib: &Path) -> Option<PathBuf> {
    let site = lib.parent()?.parent()?.parent()?;
    (site.file_name()? == "site-packages").then(|| site.to_path_buf())
}

/// Every folder under `site-packages/nvidia` that directly holds a shared library. The
/// wheels do not agree on a layout (`nvidia/cudnn/bin`, `nvidia/cu13/bin/x86_64`, ...), so
/// the tree is searched, a few levels deep, rather than guessed.
fn nvidia_lib_dirs(site: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut has_lib = false;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, out);
                }
            } else if path.extension().is_some_and(|e| e == LIB_EXT) {
                has_lib = true;
            }
        }
        if has_lib {
            out.push(dir.to_path_buf());
        }
    }
    let mut dirs = Vec::new();
    walk(&site.join("nvidia"), 4, &mut dirs);
    dirs.sort();
    dirs
}

/// Windows searches PATH, as it is at the moment of each load, for a DLL's dependencies.
#[cfg(windows)]
fn prepend_library_path(dirs: &[PathBuf]) -> Result<(), AsrError> {
    if dirs.is_empty() {
        return Ok(());
    }
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
/// nothing. Until phase 4 (Linux) preloads these libraries itself, they must already be on
/// the loader's path.
#[cfg(not(windows))]
fn prepend_library_path(dirs: &[PathBuf]) -> Result<(), AsrError> {
    if !dirs.is_empty() {
        tracing::warn!(?dirs, "CUDA libraries are not preloaded on this platform yet; put them on LD_LIBRARY_PATH");
    }
    Ok(())
}
