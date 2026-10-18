//! Parakeet TDT on ONNX Runtime: the encoder once per clip, then greedy token-and-duration
//! decoding over the encoder frames with the decoder (prediction network) and joint graphs.
//!
//! Two decoding loops make the same decisions:
//! - on the GPU, [`ParakeetTdt::greedy_batched`] calls the fused `step` graph once per
//!   emitted token, scoring every remaining frame at once;
//! - on the CPU, [`ParakeetTdt::greedy_per_frame`] calls the joint once per frame, where
//!   scoring all remaining frames each time would cost more than it saves.
//!
//! The graphs come from `utter-speech export`; `utter-speech check`
//! checks them, and the per-frame loop in Python, against Transformers token for token.

use std::path::Path;
use std::time::Instant;

use ort::ep::{self, cuda::ConvAlgorithmSearch};
use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{Tensor, TensorRef};

use crate::boost::{BoostConfig, PhraseBoost, State};
use crate::frontend::MelFrontEnd;
use crate::litert::{Accelerator, LiteRtEncoder};
use crate::runtime::WEBGPU_EP;
use crate::{AsrError, ModelConfig, Vocab};

/// Where a graph runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Cuda,
    /// onnxruntime's WebGPU plugin provider on the high-performance graphics card, the one
    /// llama-server runs on: the Vulkan backend's GPU (crate::runtime).
    WebGpu,
    Cpu,
    /// LiteRT (crate::litert), for the encoder only: Android's GPU, or its CPU on the emulator.
    LiteRt(Accelerator),
}

#[derive(Debug, Clone, Copy)]
pub struct ParakeetOptions {
    pub encoder: Device,
    /// The decoder and joint run once per emitted token and once per encoder frame: small
    /// graphs, where a GPU round trip costs more than the arithmetic. On the RTX 3090 Ti,
    /// warm, the CPU decodes in 12.6 ms p50 against 20.1 ms for the fused GPU step
    /// (docs/eval-results.md, 2026-10-09), so the CPU is the default.
    pub decoder: Device,
    /// TensorFloat-32 matrix math on the GPU: faster, slightly less precise.
    pub tf32: bool,
    /// How cuDNN picks a convolution algorithm for each new input shape.
    pub conv_search: ConvSearch,
    /// onnxruntime's memory-pattern planning. It plans for each new input shape and keeps the
    /// plan, and every utterance has a new length: off, the 8-bit build holds 1.3 GB over 240
    /// utterances instead of growing to 2.3 GB, at the same speed and with the same transcripts.
    pub memory_pattern: bool,
    /// Threads for each session's CPU work (0: onnxruntime's default, one per core). 2 with no
    /// spinning: one per core, spinning, won the speech bench back to back, but in the running
    /// app it starved everything else while the person spoke. The microphone's stream lost
    /// audio (26 gaps in three dictations, none after), and cleanup ran 15-20% slower; speech
    /// itself got faster (2026-10-12, docs/eval-results.md).
    pub cpu_threads: usize,
    /// Let idle CPU worker threads spin-wait for the next call instead of sleeping.
    pub cpu_spinning: bool,
    /// Let cuDNN take as much workspace as its fastest convolution wants (onnxruntime's
    /// default); off, it picks among algorithms that fit a small workspace.
    pub conv_max_workspace: bool,
    /// How the GPU memory pool grows: in powers of two (onnxruntime's default) or by exactly
    /// what is requested.
    pub arena_exact: bool,
    /// Put the weights in their own GPU allocations instead of the memory pool, which rounds a
    /// block up to the next power of two.
    pub weights_outside_pool: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvSearch {
    Exhaustive,
    Heuristic,
    Default,
}

impl Default for ParakeetOptions {
    fn default() -> Self {
        Self {
            encoder: Device::Cuda,
            decoder: Device::Cpu,
            tf32: true,
            conv_search: ConvSearch::Heuristic,
            memory_pattern: false,
            cpu_threads: 2,
            cpu_spinning: false,
            conv_max_workspace: true,
            arena_exact: false,
            weights_outside_pool: false,
        }
    }
}

/// Wall time of each stage of one transcription.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub frontend_s: f64,
    pub encoder_s: f64,
    pub decode_s: f64,
    pub encoder_frames: usize,
    pub joint_calls: usize,
    pub decoder_calls: usize,
}

impl Timings {
    pub fn total_s(&self) -> f64 {
        self.frontend_s + self.encoder_s + self.decode_s
    }
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub tokens: Vec<u32>,
    pub timings: Timings,
}

/// The encoder with the joint's encoder projection: features in, projected frames out.
enum Encoder {
    /// `encoder.onnx`.
    Onnx(Session),
    /// `encoder.tflite`, its compiled GPU programs cached in `gpu-cache/` beside it.
    LiteRt(Box<LiteRtEncoder>),
}

pub struct ParakeetTdt {
    cfg: ModelConfig,
    frontend: MelFrontEnd,
    vocab: Vocab,
    encoder: Encoder,
    decoder: Session,
    joint: Session,
    /// The fused decoder step and joint, used when decoding runs on the GPU.
    step: Option<Session>,
    /// Width of the projected encoder frames and the decoder output (they are added).
    hidden: usize,
    /// The dictionary's phrase boost, when there is a dictionary (see `crate::boost`).
    boost: Option<PhraseBoost>,
}

impl ParakeetTdt {
    pub fn load(dir: &Path, opts: ParakeetOptions) -> Result<Self, AsrError> {
        let t = Instant::now();
        crate::runtime::init()?;
        let runtime_ms = t.elapsed().as_secs_f64() * 1e3;
        let cfg = ModelConfig::load(dir)?;
        let frontend = MelFrontEnd::load(dir, &cfg)?;
        let vocab = Vocab::load(dir, cfg.vocab_size)?;
        // The small decoder and joint sessions are created beside the encoder's, which takes
        // seconds: starting Utter waits for the longest one only.
        let t = Instant::now();
        let (encoder, small) = std::thread::scope(|scope| {
            let small = scope.spawn(|| -> Result<_, AsrError> {
                let decoder = session(&dir.join("decoder.onnx"), opts.decoder, &opts)?;
                let joint = session(&dir.join("joint.onnx"), opts.decoder, &opts)?;
                Ok((decoder, joint, t.elapsed().as_secs_f64() * 1e3))
            });
            let encoder = match opts.encoder {
                Device::LiteRt(accelerator) => {
                    LiteRtEncoder::load(&dir.join("encoder.tflite"), accelerator, Some(&dir.join("gpu-cache")))
                        .map(|e| Encoder::LiteRt(Box::new(e)))
                }
                device => session(&dir.join("encoder.onnx"), device, &opts).map(Encoder::Onnx),
            };
            (encoder, small.join())
        });
        let encoder = encoder?;
        let encoder_ms = t.elapsed().as_secs_f64() * 1e3;
        let (decoder, joint, decoder_joint_ms) =
            small.map_err(|_| AsrError::Model("creating the decoder sessions panicked".into()))??;
        tracing::info!(runtime_ms, encoder_ms, decoder_joint_ms, "speech sessions");
        let step = match opts.decoder {
            Device::Cpu => None,
            gpu => Some(session(&dir.join("step.onnx"), gpu, &opts)?),
        };
        let hidden = cfg.decoder_hidden;
        tracing::info!(dir = %dir.display(), ?opts, "parakeet loaded");
        Ok(Self { cfg, frontend, vocab, encoder, decoder, joint, step, hidden, boost: None })
    }

    /// Boosts these phrases (the user's dictionary) in every transcription from now on; an
    /// empty list turns boosting off. Returns how many phrases could be spelled in the model's
    /// pieces. Boosting needs per-frame token scores, so it decodes frame by frame (the fused
    /// GPU step reports only the winning tokens).
    pub fn set_phrases(&mut self, phrases: &[String], cfg: BoostConfig) -> usize {
        let (boost, usable) = PhraseBoost::build(&self.vocab, phrases, cfg);
        self.boost = boost;
        tracing::info!(phrases = phrases.len(), usable, "phrase boost set");
        usable
    }

    /// Transcribes one clip of 16 kHz mono samples.
    pub fn transcribe(&mut self, audio: &[f32]) -> Result<Transcript, AsrError> {
        let mut t = Timings::default();

        if let Some(max) = self.cfg.max_encoder_frames {
            let frames = self.cfg.encoder_frames(self.frontend.frames_for(audio.len()));
            if frames > max {
                return Err(AsrError::TooLong { frames, max });
            }
        }

        let start = Instant::now();
        let features = self.frontend.compute(audio)?;
        t.frontend_s = start.elapsed().as_secs_f64();

        let start = Instant::now();
        let encoded = match &mut self.encoder {
            Encoder::Onnx(encoder) => {
                let input = TensorRef::from_array_view(([1usize, features.frames, features.mels], &features.data[..]))?;
                let outputs = encoder.run(ort::inputs!["features" => input])?;
                let (shape, data) = outputs["encoded"].try_extract_tensor::<f32>()?;
                if shape.len() != 3 || shape[2] as usize != self.hidden {
                    return Err(AsrError::Model(format!("encoder output shape {shape:?}")));
                }
                data.to_vec()
            }
            Encoder::LiteRt(encoder) => {
                if encoder.hidden() != self.hidden {
                    return Err(AsrError::Model(format!(
                        "encoder frames of {}, decoder of {}",
                        encoder.hidden(),
                        self.hidden
                    )));
                }
                encoder.encode(&features.data, features.frames)?
            }
        };
        t.encoder_s = start.elapsed().as_secs_f64();
        t.encoder_frames = encoded.len() / self.hidden;

        let start = Instant::now();
        let tokens = if self.step.is_some() && self.boost.is_none() {
            self.greedy_batched(encoded, &mut t)?
        } else {
            self.greedy_per_frame(&encoded, &mut t)?
        };
        t.decode_s = start.elapsed().as_secs_f64();

        let text = self.vocab.decode(&tokens);
        tracing::debug!(frames = t.encoder_frames, tokens = tokens.len(), total_ms = t.total_s() * 1e3, "transcribed");
        Ok(Transcript { text, tokens, timings: t })
    }

    /// Greedy TDT decoding with the fused `step` graph: one call per emitted token.
    ///
    /// Each call feeds the last emitted token (blank at the start) to the decoder and scores
    /// every frame from `frame` on with the resulting decoder output. Until a token is
    /// emitted the decoder output does not change, so walking those per-frame results on the
    /// CPU, with the same rules as [`Self::greedy_per_frame`], makes the same decisions. The
    /// encoder frames are bound once and stay on the device across calls.
    fn greedy_batched(&mut self, encoded: Vec<f32>, t: &mut Timings) -> Result<Vec<u32>, AsrError> {
        let (blank, hidden) = (self.cfg.blank_id as i64, self.hidden);
        let frames = encoded.len() / hidden;
        let layers = self.cfg.decoder_layers;
        let max_symbols = self.cfg.max_symbols_per_step;
        let step = self.step.as_mut().expect("greedy_batched needs the step graph");

        let mut binding = step.create_binding()?;
        binding.bind_input("encoded", &Tensor::from_array(([frames, hidden], encoded))?)?;
        let host = MemoryInfo::new(AllocationDevice::CPU, 0, AllocatorType::Device, MemoryType::Default)?;

        let mut h = vec![0.0f32; layers * hidden];
        let mut c = vec![0.0f32; layers * hidden];
        let mut token = blank;
        let mut tokens = Vec::new();
        let (mut frame, mut emitted) = (0usize, 0usize);
        while frame < frames {
            binding.bind_input("token", &Tensor::from_array(([1usize, 1], vec![token]))?)?;
            binding.bind_input("h", &Tensor::from_array(([layers, 1, hidden], h.clone()))?)?;
            binding.bind_input("c", &Tensor::from_array(([layers, 1, hidden], c.clone()))?)?;
            binding.bind_input("start", &Tensor::from_array(([1usize], vec![frame as i64]))?)?;
            // A bound output keeps its buffer across runs, and the scored frames shrink with
            // every call: bind the outputs afresh so each run allocates them at their size.
            binding.clear_outputs();
            for name in ["tokens", "durations", "h_out", "c_out"] {
                binding.bind_output_to_device(name, &host)?;
            }
            let outputs = step.run_binding(&binding)?;
            t.joint_calls += 1;
            t.decoder_calls += 1;
            let (_, scored) = outputs["tokens"].try_extract_tensor::<i64>()?;
            let (_, durations) = outputs["durations"].try_extract_tensor::<i64>()?;
            h.copy_from_slice(outputs["h_out"].try_extract_tensor::<f32>()?.1);
            c.copy_from_slice(outputs["c_out"].try_extract_tensor::<f32>()?.1);

            // Walk from `frame` until a token is emitted (the decoder must then run again).
            let base = frame;
            let mut emitted_token = None;
            while frame < frames {
                let i = frame - base;
                let (tok, dur) = (scored[i], self.cfg.durations[durations[i] as usize]);
                if tok == blank {
                    frame += dur.max(1);
                    emitted = 0;
                    continue;
                }
                tokens.push(tok as u32);
                emitted += 1;
                if dur > 0 {
                    frame += dur;
                    emitted = 0;
                } else if emitted >= max_symbols {
                    frame += 1;
                    emitted = 0;
                }
                emitted_token = Some(tok);
                break;
            }
            match emitted_token {
                Some(tok) => token = tok,
                None => break,
            }
        }
        Ok(tokens)
    }

    /// Greedy TDT decoding, one encoder frame per joint call. At each step the joint scores a
    /// token and a duration; a token other than blank is emitted and fed to the decoder; the
    /// frame pointer moves by the duration, by one on a blank with duration 0, and by one
    /// after `max_symbols_per_step` tokens on the same frame.
    fn greedy_per_frame(&mut self, encoded: &[f32], t: &mut Timings) -> Result<Vec<u32>, AsrError> {
        let (blank, vocab_size) = (self.cfg.blank_id, self.cfg.vocab_size);
        let frames = encoded.len() / self.hidden;
        let state_len = self.cfg.decoder_layers * self.hidden;
        let mut h = vec![0.0f32; state_len];
        let mut c = vec![0.0f32; state_len];
        let mut dec = self.decoder_step(blank as i64, &mut h, &mut c)?;
        t.decoder_calls += 1;

        let mut tokens = Vec::new();
        let (mut frame, mut emitted) = (0usize, 0usize);
        let mut boost_state = State::default();
        let mut scores = vec![0.0f32; vocab_size];
        while frame < frames {
            let enc = &encoded[frame * self.hidden..(frame + 1) * self.hidden];
            let (token, step) = {
                let enc = TensorRef::from_array_view(([1usize, self.hidden], enc))?;
                let d = TensorRef::from_array_view(([1usize, self.hidden], &dec[..]))?;
                let outputs = self.joint.run(ort::inputs!["encoded" => enc, "decoded" => d])?;
                let (_, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
                let plain = argmax(&logits[..vocab_size]);
                let token = match &self.boost {
                    // Whether to emit is the model's call alone: a bonus that beat blank would
                    // insert words nobody said. Boosting picks which token, among the rest.
                    Some(boost) if plain != blank => {
                        log_softmax(&logits[..vocab_size], &mut scores);
                        boost.apply(boost_state, &mut scores, blank);
                        scores[blank] = f32::NEG_INFINITY;
                        argmax(&scores)
                    }
                    _ => plain,
                };
                (token, self.cfg.durations[argmax(&logits[vocab_size..])])
            };
            t.joint_calls += 1;
            if token != blank {
                if let Some(boost) = &self.boost {
                    boost_state = boost.advance(boost_state, token as u32);
                }
                tokens.push(token as u32);
                emitted += 1;
                dec = self.decoder_step(token as i64, &mut h, &mut c)?;
                t.decoder_calls += 1;
            }
            if step > 0 {
                frame += step;
                emitted = 0;
            } else if token == blank || emitted >= self.cfg.max_symbols_per_step {
                frame += 1;
                emitted = 0;
            }
        }
        Ok(tokens)
    }

    /// One prediction-network step: feeds `token`, updates `h` and `c` in place, returns the
    /// decoder output.
    fn decoder_step(&mut self, token: i64, h: &mut [f32], c: &mut [f32]) -> Result<Vec<f32>, AsrError> {
        let layers = self.cfg.decoder_layers;
        let token = [token];
        let outputs = {
            let tok = TensorRef::from_array_view(([1usize, 1], &token[..]))?;
            let hs = TensorRef::from_array_view(([layers, 1, self.hidden], &*h))?;
            let cs = TensorRef::from_array_view(([layers, 1, self.hidden], &*c))?;
            self.decoder.run(ort::inputs!["token" => tok, "h" => hs, "c" => cs])?
        };
        let (_, decoded) = outputs["decoded"].try_extract_tensor::<f32>()?;
        let (_, h_out) = outputs["h_out"].try_extract_tensor::<f32>()?;
        let (_, c_out) = outputs["c_out"].try_extract_tensor::<f32>()?;
        h.copy_from_slice(h_out);
        c.copy_from_slice(c_out);
        Ok(decoded.to_vec())
    }
}

fn session(path: &Path, device: Device, opts: &ParakeetOptions) -> Result<Session, AsrError> {
    if let Device::LiteRt(_) = device {
        return Err(AsrError::Runtime(format!("{}: LiteRT runs the encoder only", path.display())));
    }
    let mut builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_memory_pattern(opts.memory_pattern)?;
    if device == Device::WebGpu {
        // The card is matched by its PCI device id, which the plugin reports as DXGI does.
        // Plugin 0.4.0 then runs on Dawn's high-performance adapter whatever device it is
        // given (its LUID options too, 2026-10-13): the same card, the first in DXGI's
        // high-performance order, unless UTTER_GPU_INDEX picks another for testing. Like CUDA,
        // no card is an error, never a silent fall back to the CPU.
        let adapter = utter_platform::gpu::high_performance_adapter()
            .ok_or_else(|| AsrError::Runtime("no graphics card found".into()))?;
        let env = ort::environment::Environment::current()?;
        // A device id of 0 is a GPU with no PCI id (Apple Silicon's, the only one there).
        let devices: Vec<_> = env
            .devices()
            .filter(|d| {
                d.ep().is_ok_and(|ep| ep == WEBGPU_EP)
                    && (adapter.device_id == 0 || d.hardware_device().id() == adapter.device_id)
            })
            .take(1)
            .collect();
        if devices.is_empty() {
            return Err(AsrError::Runtime(format!("WebGPU does not offer {}", adapter.name)));
        }
        builder = builder.with_devices(devices, None)?;
    }
    if device == Device::Cuda {
        // A failed CUDA registration is an error, never a silent fall back to the CPU.
        let search = match opts.conv_search {
            ConvSearch::Exhaustive => ConvAlgorithmSearch::Exhaustive,
            ConvSearch::Heuristic => ConvAlgorithmSearch::Heuristic,
            ConvSearch::Default => ConvAlgorithmSearch::Default,
        };
        // The memory pool keeps onnxruntime's default growth (powers of two). Growing by exactly
        // what is requested measured worse, 3.1 GB against 2.3 GB: with a new input length on
        // every utterance, exact-size blocks fragment instead of being reused (2026-10-09).
        let arena = if opts.arena_exact {
            ep::ArenaExtendStrategy::SameAsRequested
        } else {
            ep::ArenaExtendStrategy::NextPowerOfTwo
        };
        let cuda = ep::CUDA::default()
            .with_conv_algorithm_search(search)
            .with_tf32(opts.tf32)
            .with_conv_max_workspace(opts.conv_max_workspace)
            .with_arena_extend_strategy(arena);
        builder = builder.with_execution_providers([cuda.build().error_on_failure()])?;
        if opts.weights_outside_pool {
            builder = builder.with_config_entry("session.use_device_allocator_for_initializers", "1")?;
        }
    }
    // Every session has a CPU thread pool (a GPU session's runs its CPU-side nodes).
    if opts.cpu_threads > 0 {
        builder = builder.with_intra_threads(opts.cpu_threads)?;
    }
    builder = builder.with_intra_op_spinning(opts.cpu_spinning)?;
    let session = builder.commit_from_file(path).map_err(|e| AsrError::Model(format!("{}: {e}", path.display())))?;
    // After the session, so after the execution provider's library has loaded.
    crate::runtime::skip_teardown_at_exit();
    Ok(session)
}

fn argmax(values: &[f32]) -> usize {
    let mut best = 0;
    for (i, &v) in values.iter().enumerate() {
        if v > values[best] {
            best = i;
        }
    }
    best
}

/// `out[i] = logits[i] - log(sum(exp(logits)))`, computed stably.
fn log_softmax(logits: &[f32], out: &mut [f32]) {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let log_sum = logits.iter().map(|&x| (x - max).exp()).sum::<f32>().ln() + max;
    for (o, &x) in out.iter_mut().zip(logits) {
        *o = x - log_sum;
    }
}

#[cfg(test)]
mod tests {
    use super::{argmax, log_softmax};

    #[test]
    fn log_softmax_is_normalized_and_keeps_the_order() {
        let mut out = [0.0f32; 3];
        log_softmax(&[1.0, 3.0, 2.0], &mut out);
        let total: f32 = out.iter().map(|x| x.exp()).sum();
        assert!((total - 1.0).abs() < 1e-5);
        assert_eq!(argmax(&out), 1);
    }

    #[test]
    fn argmax_takes_the_first_of_equal_maxima() {
        // numpy's argmax, which the Python reference uses, does the same.
        assert_eq!(argmax(&[0.1, 0.9, 0.9, 0.2]), 1);
    }
}
