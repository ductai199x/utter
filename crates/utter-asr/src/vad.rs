//! Voice activity detection: Silero VAD (the `silero-vad` component) on the CPU, and the
//! segmenter that turns its speech probabilities into cut points.
//!
//! Silero takes 512 new samples (32 ms at 16 kHz) with the 64 samples before them as context,
//! and a recurrent state [2, 1, 128]; it returns the probability that the frame is speech.
//! The same framing as onnx-asr's reference implementation.

use std::path::Path;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;

use crate::AsrError;

/// Samples per VAD frame at 16 kHz (32 ms).
pub const FRAME: usize = 512;
const CONTEXT: usize = 64;
const STATE_LEN: usize = 2 * 128;

pub const COMPONENT: &str = "silero-vad";

pub struct SileroVad {
    session: Session,
    state: Vec<f32>,
    /// The last CONTEXT samples of the previous frame.
    context: Vec<f32>,
    /// Samples waiting for a full frame.
    pending: Vec<f32>,
    input: Vec<f32>,
}

impl SileroVad {
    /// Loads the installed `silero-vad` component.
    pub fn load_installed() -> Result<Self, AsrError> {
        let dir = utter_runtime::component(COMPONENT)
            .and_then(|c| c.installed_dir())
            .map_err(|e| AsrError::Runtime(e.to_string()))?;
        Self::load(&dir.join("silero_vad.onnx"))
    }

    pub fn load(path: &Path) -> Result<Self, AsrError> {
        crate::runtime::init()?;
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?
            .commit_from_file(path)
            .map_err(|e| AsrError::Model(format!("{}: {e}", path.display())))?;
        Ok(Self {
            session,
            state: vec![0.0; STATE_LEN],
            context: vec![0.0; CONTEXT],
            pending: Vec::with_capacity(FRAME * 4),
            input: vec![0.0; CONTEXT + FRAME],
        })
    }

    /// Forgets everything: the next sample starts a new stream.
    pub fn reset(&mut self) {
        self.state.fill(0.0);
        self.context.fill(0.0);
        self.pending.clear();
    }

    /// Feeds samples and returns one speech probability per completed 512-sample frame.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>, AsrError> {
        self.pending.extend_from_slice(samples);
        let mut probs = Vec::with_capacity(self.pending.len() / FRAME);
        let mut used = 0;
        while self.pending.len() - used >= FRAME {
            self.input[..CONTEXT].copy_from_slice(&self.context);
            self.input[CONTEXT..].copy_from_slice(&self.pending[used..used + FRAME]);
            probs.push(self.run_frame()?);
            self.context.copy_from_slice(&self.input[FRAME..]);
            used += FRAME;
        }
        self.pending.drain(..used);
        Ok(probs)
    }

    fn run_frame(&mut self) -> Result<f32, AsrError> {
        let sr = [16_000i64];
        let outputs = {
            let input = TensorRef::from_array_view(([1usize, CONTEXT + FRAME], &self.input[..]))?;
            let state = TensorRef::from_array_view(([2usize, 1, 128], &self.state[..]))?;
            let sr = TensorRef::from_array_view(([1usize], &sr[..]))?;
            self.session.run(ort::inputs!["input" => input, "state" => state, "sr" => sr])?
        };
        let (_, prob) = outputs["output"].try_extract_tensor::<f32>()?;
        let (_, state) = outputs["stateN"].try_extract_tensor::<f32>()?;
        let p = prob[0];
        self.state.copy_from_slice(state);
        Ok(p)
    }
}

/// Where the segmenter cuts.
#[derive(Debug, Clone, Copy)]
pub struct SegmenterConfig {
    /// A frame at or above this probability is speech.
    pub speech_threshold: f32,
    /// A frame below this is silence (hysteresis: between the two, the state holds).
    pub silence_threshold: f32,
    /// Silence after speech, in frames, before a cut (12 frames = 384 ms).
    pub min_silence_frames: usize,
    /// The longest segment, in frames; a longer one is cut at its quietest frame.
    pub max_segment_frames: usize,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            speech_threshold: 0.5,
            silence_threshold: 0.35,
            min_silence_frames: 12,
            // 30 s: under the encoder's 41 s limit, with room for the pre-roll.
            max_segment_frames: 30 * 16_000 / FRAME,
        }
    }
}

/// Turns per-frame speech probabilities into cut points: the middle of each pause long enough
/// after speech, so a segment never splits a word, and a forced cut at the quietest frame when
/// a segment would grow past the maximum.
#[derive(Debug, Clone)]
pub struct Segmenter {
    cfg: SegmenterConfig,
    /// Frames seen so far.
    frame: usize,
    /// Frame index where the current segment starts.
    seg_start: usize,
    in_speech: bool,
    /// Speech seen in the current segment.
    had_speech: bool,
    /// Where the current run of silence began, if in silence after speech.
    silence_start: Option<usize>,
    /// Quietest frame of the current segment, for a forced cut.
    quietest: (usize, f32),
}

impl Segmenter {
    pub fn new(cfg: SegmenterConfig) -> Self {
        Self {
            cfg,
            frame: 0,
            seg_start: 0,
            in_speech: false,
            had_speech: false,
            silence_start: None,
            quietest: (0, f32::INFINITY),
        }
    }

    /// Feeds probabilities; returns the cut points they complete, as frame indices: a
    /// segment ends (exclusive) and the next begins at each.
    pub fn push(&mut self, probs: &[f32]) -> Vec<usize> {
        let mut cuts = Vec::new();
        for &p in probs {
            let i = self.frame;
            self.frame += 1;
            if p < self.quietest.1 && i > self.seg_start {
                self.quietest = (i, p);
            }
            if p >= self.cfg.speech_threshold {
                self.in_speech = true;
                self.had_speech = true;
                self.silence_start = None;
            } else if p < self.cfg.silence_threshold {
                if self.in_speech || (self.had_speech && self.silence_start.is_none()) {
                    self.silence_start = Some(i);
                }
                self.in_speech = false;
                if let Some(s) = self.silence_start
                    && self.had_speech
                    && i + 1 - s >= self.cfg.min_silence_frames
                {
                    let cut = s + (i + 1 - s) / 2;
                    cuts.push(cut);
                    self.start_segment(cut);
                    continue;
                }
            }
            if self.frame - self.seg_start >= self.cfg.max_segment_frames {
                let cut = self.quietest.0.max(self.seg_start + 1);
                cuts.push(cut);
                self.start_segment(cut);
                self.had_speech = self.in_speech;
            }
        }
        cuts
    }

    fn start_segment(&mut self, at: usize) {
        self.seg_start = at;
        self.had_speech = false;
        self.silence_start = None;
        self.quietest = (at, f32::INFINITY);
    }

    /// Whether the current (open) segment has any speech yet.
    pub fn has_speech(&self) -> bool {
        self.had_speech
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg() -> Segmenter {
        Segmenter::new(SegmenterConfig { min_silence_frames: 4, max_segment_frames: 1000, ..Default::default() })
    }

    #[test]
    fn a_pause_after_speech_is_cut_in_its_middle() {
        let mut s = seg();
        // 5 speech frames, then 4 silent ones (frames 5..9): cut at 5 + 4/2 = 7.
        let probs = [0.9, 0.9, 0.9, 0.9, 0.9, 0.1, 0.1, 0.1, 0.1];
        assert_eq!(s.push(&probs), vec![7]);
    }

    #[test]
    fn silence_before_any_speech_is_never_cut() {
        let mut s = seg();
        assert!(s.push(&[0.0; 50]).is_empty());
        assert!(!s.has_speech());
    }

    #[test]
    fn a_short_pause_does_not_cut() {
        let mut s = seg();
        let probs = [0.9, 0.9, 0.1, 0.1, 0.9, 0.9];
        assert!(s.push(&probs).is_empty());
        assert!(s.has_speech());
    }

    #[test]
    fn hysteresis_between_thresholds_keeps_speech() {
        let mut s = seg();
        // 0.4 is between silence (0.35) and speech (0.5): not silence.
        let probs = [0.9, 0.4, 0.4, 0.4, 0.4, 0.4, 0.9];
        assert!(s.push(&probs).is_empty());
    }

    #[test]
    fn frames_arriving_in_pieces_cut_the_same() {
        let probs = [0.9, 0.9, 0.9, 0.1, 0.1, 0.1, 0.1, 0.9, 0.9, 0.1, 0.1, 0.1, 0.1];
        let mut whole = seg();
        let all = whole.push(&probs);
        let mut pieces = seg();
        let mut got = Vec::new();
        for chunk in probs.chunks(3) {
            got.extend(pieces.push(chunk));
        }
        assert_eq!(all, got);
        assert_eq!(all, vec![5, 11]);
    }

    #[test]
    fn a_segment_too_long_is_cut_at_its_quietest_frame() {
        let mut s =
            Segmenter::new(SegmenterConfig { min_silence_frames: 100, max_segment_frames: 10, ..Default::default() });
        let mut probs = vec![0.9; 10];
        probs[6] = 0.6; // the quietest, still speech
        assert_eq!(s.push(&probs), vec![6]);
    }
}
