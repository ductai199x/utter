//! Speech recognition for Utter.
//!
//! The model is chosen by measurement (DESIGN.md section 6, docs/eval-results.md): English
//! runs parakeet-ultra's weights, exported to ONNX, in [`ParakeetTdt`].

pub mod boost;
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

/// The speech model's two builds: the same weights, run where the tier has room for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeechModel {
    /// The encoder in float16 on the GPU: about 2.3 GB of GPU memory, 35-55 ms a clip.
    #[default]
    Gpu,
    /// The encoder's weights in 8 bits on the CPU (export/quantize_encoder.py): no GPU memory,
    /// for the extra light tier's 2.5 GB budget, at a few hundred milliseconds more a clip.
    Cpu,
}

impl SpeechModel {
    pub fn component(self) -> &'static str {
        match self {
            SpeechModel::Gpu => MODEL_COMPONENT,
            SpeechModel::Cpu => "parakeet-ultra-onnx-w8",
        }
    }

    /// How this build runs: where the encoder goes.
    pub fn options(self) -> ParakeetOptions {
        match self {
            SpeechModel::Gpu => ParakeetOptions::default(),
            SpeechModel::Cpu => ParakeetOptions { encoder: Device::Cpu, ..ParakeetOptions::default() },
        }
    }

    /// The installed model's folder, or an error naming how to install it.
    pub fn installed_dir(self) -> Result<PathBuf, AsrError> {
        let c = utter_runtime::component(self.component()).map_err(|e| AsrError::Runtime(e.to_string()))?;
        c.installed_dir().map_err(|e| AsrError::Runtime(e.to_string()))
    }
}

/// The installed GPU speech model's folder, or an error naming how to install it.
pub fn installed_model_dir() -> Result<PathBuf, AsrError> {
    SpeechModel::Gpu.installed_dir()
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
