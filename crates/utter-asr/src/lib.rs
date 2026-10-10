//! Speech recognition for Utter.
//!
//! The model is chosen by measurement (DESIGN.md section 6, docs/eval-results.md): English
//! runs parakeet-ultra's weights, exported to ONNX, in [`ParakeetTdt`].

mod config;
pub mod frontend;
pub mod parakeet;
pub mod runtime;
pub mod vad;
mod vocab;

use std::path::{Path, PathBuf};

pub use config::ModelConfig;
pub use parakeet::{ConvSearch, Device, ParakeetOptions, ParakeetTdt, Timings, Transcript};
pub use vocab::Vocab;

#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("model: {0}")]
    Model(String),
    #[error("onnxruntime: {0}")]
    Runtime(String),
    #[error("clip too short to transcribe: {samples} samples")]
    TooShort { samples: usize },
    #[error("clip too long for one pass: {frames} encoder frames, at most {max}; split it at a pause")]
    TooLong { frames: usize, max: usize },
}

/// The runtime component holding the English speech model (crates/utter-runtime/components.json).
pub const MODEL_COMPONENT: &str = "parakeet-ultra-onnx-fp16";

/// The installed speech model's folder, or an error naming how to install it.
pub fn installed_model_dir() -> Result<PathBuf, AsrError> {
    let c = utter_runtime::component(MODEL_COMPONENT).map_err(|e| AsrError::Runtime(e.to_string()))?;
    c.installed_dir().map_err(|e| AsrError::Runtime(e.to_string()))
}

impl AsrError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io { path: path.to_path_buf(), source }
    }
}

impl<T> From<ort::Error<T>> for AsrError {
    fn from(e: ort::Error<T>) -> Self {
        Self::Runtime(e.to_string())
    }
}
