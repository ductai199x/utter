//! The audio front end: 16 kHz samples to normalized log-mel features, exactly as
//! Transformers' `ParakeetFeatureExtractor` computes them for one clip.
//!
//! Steps: pre-emphasis; a centered STFT (zero padding of n_fft/2 on both sides, a symmetric
//! Hann window of `win_length` centered in `n_fft`); the power spectrum; the Slaney mel
//! filterbank exported with the model; `ln(mel + 2^-24)`; then per-mel normalization to zero
//! mean and unit (unbiased) standard deviation over the clip's valid frames.
//!
//! Only the valid frames are produced (`samples / hop`): the extractor computes one more
//! frame and masks it out, and the encoder is exported to take exactly the valid ones.
//! Parity with Transformers is tested against a fixture it wrote (`tests/frontend.rs`).

use std::path::Path;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

use crate::{AsrError, ModelConfig};

/// Log-mel features of one clip, row-major `[frames][mels]`.
#[derive(Debug, Clone)]
pub struct Features {
    pub data: Vec<f32>,
    pub frames: usize,
    pub mels: usize,
}

pub struct MelFrontEnd {
    n_fft: usize,
    hop: usize,
    mels: usize,
    preemphasis: f32,
    log_guard: f32,
    epsilon: f32,
    /// The analysis window, `n_fft` long: the Hann window centered, zeros around it.
    window: Vec<f32>,
    /// Each mel filter's nonzero span: (first bin, weights). Slaney filters are triangles a
    /// few bins wide, so this is about 2 x 257 multiplies per frame instead of 128 x 257.
    filters: Vec<(usize, Vec<f32>)>,
    fft: Arc<dyn RealToComplex<f32>>,
}

impl MelFrontEnd {
    pub fn load(dir: &Path, cfg: &ModelConfig) -> Result<Self, AsrError> {
        let path = dir.join("mel_filters.f32");
        let bytes = std::fs::read(&path).map_err(|e| AsrError::io(&path, e))?;
        let bins = cfg.n_fft / 2 + 1;
        if bytes.len() != cfg.n_mels * bins * 4 {
            return Err(AsrError::Model(format!(
                "{}: {} bytes, expected {} mels x {bins} bins",
                path.display(),
                bytes.len(),
                cfg.n_mels
            )));
        }
        let filters = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        Ok(Self::new(cfg, filters))
    }

    /// `filters` is the dense filterbank, `[mels][n_fft / 2 + 1]` row-major.
    pub fn new(cfg: &ModelConfig, filters: Vec<f32>) -> Self {
        let (n_fft, win) = (cfg.n_fft, cfg.win_length);
        let bins = n_fft / 2 + 1;
        let filters = filters
            .chunks_exact(bins)
            .map(|row| match row.iter().position(|&w| w != 0.0) {
                Some(first) => {
                    let last = row.iter().rposition(|&w| w != 0.0).unwrap_or(first);
                    (first, row[first..=last].to_vec())
                }
                None => (0, Vec::new()),
            })
            .collect();
        // torch.hann_window(win, periodic=False), placed in the middle of n_fft as torch.stft does.
        let offset = (n_fft - win) / 2;
        let mut window = vec![0.0f32; n_fft];
        for k in 0..win {
            let phase = 2.0 * std::f64::consts::PI * k as f64 / (win - 1) as f64;
            window[offset + k] = (0.5 - 0.5 * phase.cos()) as f32;
        }
        Self {
            n_fft,
            hop: cfg.hop_length,
            mels: cfg.n_mels,
            preemphasis: cfg.preemphasis,
            log_guard: cfg.log_guard,
            epsilon: cfg.norm_epsilon,
            window,
            filters,
            fft: RealFftPlanner::<f32>::new().plan_fft_forward(n_fft),
        }
    }

    /// The number of valid frames for a clip of `samples`.
    pub fn frames_for(&self, samples: usize) -> usize {
        samples / self.hop
    }

    pub fn compute(&self, audio: &[f32]) -> Result<Features, AsrError> {
        let frames = self.frames_for(audio.len());
        // Normalization divides by frames - 1: a clip needs two frames (20 ms at 16 kHz).
        if frames < 2 {
            return Err(AsrError::TooShort { samples: audio.len() });
        }
        let pad = self.n_fft / 2;
        let bins = self.n_fft / 2 + 1;

        // Pre-emphasis, written straight into the zero-padded signal the centered STFT reads.
        let mut padded = vec![0.0f32; audio.len() + 2 * pad];
        padded[pad] = audio[0];
        for i in 1..audio.len() {
            padded[pad + i] = audio[i] - self.preemphasis * audio[i - 1];
        }

        let mut frame = self.fft.make_input_vec();
        let mut spectrum = self.fft.make_output_vec();
        let mut scratch = self.fft.make_scratch_vec();
        let mut power = vec![0.0f32; bins];
        let mut data = vec![0.0f32; frames * self.mels];
        for f in 0..frames {
            let start = f * self.hop;
            for (k, x) in frame.iter_mut().enumerate() {
                *x = padded[start + k] * self.window[k];
            }
            self.fft
                .process_with_scratch(&mut frame, &mut spectrum, &mut scratch)
                .map_err(|e| AsrError::Model(format!("FFT: {e}")))?;
            for (p, c) in power.iter_mut().zip(&spectrum) {
                *p = c.re * c.re + c.im * c.im;
            }
            let row = &mut data[f * self.mels..(f + 1) * self.mels];
            for (out, (first, weights)) in row.iter_mut().zip(&self.filters) {
                let energy: f32 = weights.iter().zip(&power[*first..]).map(|(w, p)| w * p).sum();
                *out = (energy + self.log_guard).ln();
            }
        }

        // Per-mel normalization over the valid frames (statistics in f64).
        for m in 0..self.mels {
            let mean = (0..frames).map(|f| data[f * self.mels + m] as f64).sum::<f64>() / frames as f64;
            let var =
                (0..frames).map(|f| (data[f * self.mels + m] as f64 - mean).powi(2)).sum::<f64>() / (frames - 1) as f64;
            let denom = var.sqrt() + self.epsilon as f64;
            for f in 0..frames {
                let x = &mut data[f * self.mels + m];
                *x = ((*x as f64 - mean) / denom) as f32;
            }
        }
        Ok(Features { data, frames, mels: self.mels })
    }
}
