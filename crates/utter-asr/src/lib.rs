//! Speech recognition for Utter.
//!
//! The model is chosen by measurement (DESIGN.md section 6, docs/eval-results.md): English
//! runs parakeet-ultra's weights, exported to ONNX, in [`ParakeetTdt`].

pub mod boost;
mod config;
pub mod frontend;
pub mod litert;
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

/// The speech model's two builds, both on the GPU (2026-10-11: everything on the GPU),
/// with the same transcripts on the eval clips. GPU memory after load and 240 utterances, the
/// 10 longest eval clips among them, with each build's settings (docs/eval-results.md):
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeechModel {
    /// The encoder in float16: about 2.0 GB, 21-23 ms of encoder a clip. Kept for comparisons.
    #[default]
    Fp16,
    /// The encoder's weights in 8 bits, float16 activations (export/quantize_encoder.py): about
    /// 1.3 GB, some 5 ms slower, the same transcripts. What both tiers run.
    W8,
}

impl SpeechModel {
    pub fn component(self) -> &'static str {
        match self {
            SpeechModel::Fp16 => MODEL_COMPONENT,
            SpeechModel::W8 => "parakeet-ultra-onnx-w8",
        }
    }

    /// How this build runs, memory-pattern planning off for both ([`ParakeetOptions`]). The
    /// float16 weights go outside the memory pool, which would round their 1.2 GB up to 2 GB;
    /// the 8-bit ones stay in it, where the rounding leaves room the activations then use. On the
    /// Vulkan backend the encoder runs on WebGPU, which runs float16 faster than 8 bits
    /// (docs/plan-vulkan.md); utter_pipeline::speech_for picks Fp16 there.
    pub fn options(self) -> ParakeetOptions {
        let encoder = match utter_runtime::backend() {
            utter_runtime::Backend::Cuda => Device::Cuda,
            utter_runtime::Backend::Vulkan | utter_runtime::Backend::Metal => Device::WebGpu,
            utter_runtime::Backend::Cpu => Device::Cpu,
        };
        let base = ParakeetOptions { encoder, ..ParakeetOptions::default() };
        match self {
            SpeechModel::Fp16 => ParakeetOptions { weights_outside_pool: true, ..base },
            SpeechModel::W8 => base,
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
    SpeechModel::Fp16.installed_dir()
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
