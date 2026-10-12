//! The exported model's `config.json` (written by `utter-speech export`).

use std::path::Path;

use serde::Deserialize;

use crate::AsrError;

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub weights: String,
    pub vocab_size: usize,
    pub blank_id: usize,
    pub durations: Vec<usize>,
    pub max_symbols_per_step: usize,
    pub decoder_layers: usize,
    pub decoder_hidden: usize,
    pub sample_rate: u32,
    pub n_fft: usize,
    pub win_length: usize,
    pub hop_length: usize,
    pub n_mels: usize,
    pub preemphasis: f32,
    pub log_guard: f32,
    pub norm_epsilon: f32,
    /// The longest clip the encoder takes, in encoder frames (None: no limit). Set when the
    /// export gives the depthwise convolutions a fixed input length.
    #[serde(default)]
    pub max_encoder_frames: Option<usize>,
    #[serde(default = "default_subsampling")]
    pub subsampling_factor: usize,
}

fn default_subsampling() -> usize {
    8
}

impl ModelConfig {
    /// Encoder frames for `feature_frames` mel frames: each stride-2 convolution of the
    /// subsampling (kernel 3, padding 1) maps n to (n - 1) / 2 + 1.
    pub fn encoder_frames(&self, feature_frames: usize) -> usize {
        let mut n = feature_frames;
        let mut f = self.subsampling_factor;
        while f > 1 {
            n = (n.saturating_sub(1)) / 2 + 1;
            f /= 2;
        }
        n
    }
}

impl ModelConfig {
    pub fn load(dir: &Path) -> Result<Self, AsrError> {
        let path = dir.join("config.json");
        let text = std::fs::read_to_string(&path).map_err(|e| AsrError::io(&path, e))?;
        serde_json::from_str(&text).map_err(|e| AsrError::Model(format!("{}: {e}", path.display())))
    }
}
