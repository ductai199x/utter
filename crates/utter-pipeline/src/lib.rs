//! The dictation pipeline (DESIGN.md section 4): hotkey -> microphone -> segments cut at
//! pauses and transcribed while the user speaks -> the last segment at key up -> the cleanup
//! gate -> cleanup -> insertion into the window that had the focus at key down.
//!
//! Threads: the hotkey's (utter-platform), the microphone's (utter-audio), a speech worker
//! that owns the recognizer, the cleanup model's server process, and the pipeline loop, which
//! polls the microphone every [`POLL`] while listening. Every dictation's stage times are
//! measured from key up, the moment the user starts waiting.

pub mod gate;

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use utter_asr::vad::{FRAME, Segmenter, SegmenterConfig, SileroVad};
use utter_asr::{ParakeetOptions, ParakeetTdt};
use utter_audio::{MicConfig, Microphone};
use utter_llm::cleanup::{self, Tier};
use utter_llm::{ChatParams, LlamaServer};
use utter_platform::focus::{self, Window};
use utter_platform::hotkey::{self, Action, HotkeyConfig};
use utter_platform::insert::{self, PasteOptions};

/// How often the pipeline reads the microphone while listening.
pub const POLL: Duration = Duration::from_millis(30);
/// While listening, a recognizer pass at least this often keeps the GPU's clocks up
/// (docs/eval-results.md, "Idle GPU").
const KEEPALIVE: Duration = Duration::from_millis(150);
/// Audio the worker may still hold at key up: a resampler chunk (21 ms at 48 kHz) and change.
const SETTLE: Duration = Duration::from_millis(25);

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("{0}")]
    Audio(#[from] utter_audio::AudioError),
    #[error("{0}")]
    Asr(#[from] utter_asr::AsrError),
    #[error("{0}")]
    Llm(#[from] utter_llm::LlmError),
    #[error("{0}")]
    Hotkey(#[from] hotkey::HotkeyError),
    #[error("speech worker stopped")]
    WorkerGone,
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub tier: Tier,
    pub dictionary: Vec<String>,
    pub mic: MicConfig,
    pub hotkey: HotkeyConfig,
    /// Skip the cleanup model entirely (raw transcripts).
    pub no_cleanup: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            tier: Tier::Light,
            dictionary: Vec::new(),
            mic: MicConfig::default(),
            hotkey: HotkeyConfig::default(),
            no_cleanup: false,
        }
    }
}

/// What happened in one dictation, with times measured from key up.
#[derive(Debug, Clone)]
pub struct Dictation {
    pub raw: String,
    pub text: String,
    pub target: Option<Window>,
    pub audio_s: f64,
    pub segments: usize,
    pub gate: Option<&'static str>,
    pub inserted: Result<(), String>,
    /// Key up -> every segment transcribed.
    pub asr_ms: f64,
    /// Cleanup request time (0 when the gate skipped it).
    pub cleanup_ms: f64,
    /// Key up -> paste sent.
    pub total_ms: f64,
}

enum Job {
    /// A recognizer pass on a short silent clip, only to keep the GPU busy.
    KeepAlive,
    Segment {
        generation: u64,
        index: usize,
        audio: Vec<f32>,
    },
}

struct Done {
    generation: u64,
    index: usize,
    text: String,
}

/// Owns the recognizer; runs jobs in order.
fn speech_worker(mut asr: ParakeetTdt, jobs: Receiver<Job>, done: Sender<Done>) {
    let silence = vec![0.0f32; 8_000];
    while let Ok(mut job) = jobs.recv() {
        // A keep-alive with real work queued behind it is skipped: it would only delay the
        // segment, possibly the last one, which the user is waiting for.
        while matches!(job, Job::KeepAlive) {
            match jobs.try_recv() {
                Ok(next) => job = next,
                Err(_) => break,
            }
        }
        match job {
            Job::KeepAlive => {
                let _ = asr.transcribe(&silence);
            }
            Job::Segment { generation, index, audio } => {
                let text = match asr.transcribe(&audio) {
                    Ok(t) => t.text,
                    Err(utter_asr::AsrError::TooShort { .. }) => String::new(),
                    Err(e) => {
                        tracing::warn!("segment {index}: {e}");
                        String::new()
                    }
                };
                if done.send(Done { generation, index, text }).is_err() {
                    break;
                }
            }
        }
    }
}

/// Where a dictation's audio comes from: the microphone, or a recording replayed in real time.
pub trait AudioSource {
    fn start(&mut self);
    /// Samples since the last call.
    fn take_new(&mut self) -> Vec<f32>;
    /// Ends the utterance; returns all of it.
    fn stop(&mut self, settle: Duration) -> Vec<f32>;
    /// For a replay: the recording has all been delivered (the key goes up).
    fn ended(&self) -> bool {
        false
    }
}

struct MicSource<'a>(&'a Microphone);

impl AudioSource for MicSource<'_> {
    fn start(&mut self) {
        self.0.start();
    }
    fn take_new(&mut self) -> Vec<f32> {
        self.0.take_new()
    }
    fn stop(&mut self, settle: Duration) -> Vec<f32> {
        self.0.stop(settle)
    }
}

/// A recording delivered at wall-clock pace, as the microphone would deliver it.
pub struct ReplaySource<'a> {
    audio: &'a [f32],
    started: Option<Instant>,
    given: usize,
}

impl<'a> ReplaySource<'a> {
    pub fn new(audio: &'a [f32]) -> Self {
        Self { audio, started: None, given: 0 }
    }
}

impl AudioSource for ReplaySource<'_> {
    fn start(&mut self) {
        self.started = Some(Instant::now());
        self.given = 0;
    }
    fn take_new(&mut self) -> Vec<f32> {
        let elapsed = self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        let due = ((elapsed * utter_audio::SAMPLE_RATE as f64) as usize).min(self.audio.len());
        let new = self.audio[self.given..due].to_vec();
        self.given = due;
        new
    }
    fn stop(&mut self, _settle: Duration) -> Vec<f32> {
        self.audio[..self.given].to_vec()
    }
    fn ended(&self) -> bool {
        self.given >= self.audio.len()
    }
}

pub struct Pipeline {
    mic: Microphone,
    engine: Engine,
}

/// Everything but the microphone: voice activity detection, the speech worker, the cleanup
/// model. Separate so a dictation can borrow the microphone and the engine at once.
struct Engine {
    cfg: PipelineConfig,
    vad: SileroVad,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    llm: Option<LlamaServer>,
    generation: u64,
}

impl Pipeline {
    /// Loads everything up front (models, server, microphone) and warms it, so the first
    /// dictation is as fast as the rest.
    pub fn start(cfg: PipelineConfig) -> Result<Self, PipelineError> {
        let model_dir = utter_asr::installed_model_dir()?;
        let mut asr = ParakeetTdt::load(&model_dir, ParakeetOptions::default())?;
        for _ in 0..3 {
            asr.transcribe(&vec![0.0f32; 16_000])?;
        }
        let (job_tx, job_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("utter-speech".into())
            .spawn(move || speech_worker(asr, job_rx, done_tx))
            .map_err(|_| PipelineError::WorkerGone)?;

        let llm = if cfg.no_cleanup {
            None
        } else {
            let server = LlamaServer::start(&cleanup::server_options(cfg.tier)?)?;
            // The first request after loading is slow; pay for it now.
            let warm = "Um, this is a warm-up, no, a second warm-up.";
            server.chat(&cleanup::messages(warm, &cfg.dictionary), &ChatParams::default())?;
            Some(server)
        };
        let vad = SileroVad::load_installed()?;
        let mic = Microphone::open(&cfg.mic)?;
        Ok(Self { mic, engine: Engine { cfg, vad, jobs: job_tx, done: done_rx, llm, generation: 0 } })
    }

    pub fn microphone(&self) -> &Microphone {
        &self.mic
    }

    /// Runs dictations until the hotkey listener ends; `on_done` sees each one.
    pub fn run(&mut self, mut on_done: impl FnMut(&Dictation)) -> Result<(), PipelineError> {
        let listener = hotkey::listen(self.engine.cfg.hotkey)?;
        loop {
            let Some((action, _at)) = listener.recv_timeout(Duration::from_secs(3600)) else { continue };
            if action != Action::Start {
                continue;
            }
            let target = focus::foreground();
            let mut source = MicSource(&self.mic);
            if let Some(d) = self.engine.dictate(&mut source, |t| listener.recv_timeout(t), target, true)? {
                on_done(&d);
            }
        }
    }

    /// One dictation of a recording, fed in at real-time pace as if spoken, with the key
    /// released when it ends: the live code path end to end, without insertion. For the bench.
    pub fn replay(&mut self, audio: &[f32]) -> Result<Dictation, PipelineError> {
        let mut source = ReplaySource::new(audio);
        let d = self.engine.dictate(
            &mut source,
            |t| {
                std::thread::sleep(t);
                None
            },
            None,
            false,
        )?;
        Ok(d.expect("a replay is never cancelled"))
    }
}

impl Engine {
    /// One dictation, from the Start action: returns `None` if it was cancelled.
    fn dictate(
        &mut self,
        source: &mut dyn AudioSource,
        mut next_action: impl FnMut(Duration) -> Option<(Action, Instant)>,
        target: Option<Window>,
        do_insert: bool,
    ) -> Result<Option<Dictation>, PipelineError> {
        self.generation += 1;
        let generation = self.generation;
        source.start();
        self.vad.reset();
        let mut segmenter = Segmenter::new(SegmenterConfig::default());
        let mut audio: Vec<f32> = Vec::new();
        let mut last_cut = 0usize;
        let mut sent = 0usize;
        let mut last_job = Instant::now();
        let _ = self.jobs.send(Job::KeepAlive);

        let key_up = loop {
            if source.ended() {
                break Instant::now();
            }
            match next_action(POLL) {
                Some((Action::Finish, at)) => break at,
                Some((Action::Cancel, _)) => {
                    source.stop(Duration::ZERO);
                    return Ok(None);
                }
                // Hands-free: keep listening until Finish.
                Some((Action::HandsFree | Action::Start, _)) | None => {}
            }
            let new = source.take_new();
            if new.is_empty() {
                continue;
            }
            audio.extend_from_slice(&new);
            let probs = self.vad.push(&new)?;
            for cut in segmenter.push(&probs) {
                let cut_sample = (cut * FRAME).min(audio.len());
                if cut_sample > last_cut {
                    let segment = audio[last_cut..cut_sample].to_vec();
                    self.jobs
                        .send(Job::Segment { generation, index: sent, audio: segment })
                        .map_err(|_| PipelineError::WorkerGone)?;
                    sent += 1;
                    last_cut = cut_sample;
                    last_job = Instant::now();
                }
            }
            if last_job.elapsed() >= KEEPALIVE {
                let _ = self.jobs.send(Job::KeepAlive);
                last_job = Instant::now();
            }
        };

        // Key up: the user is waiting from here on.
        let rest = source.stop(SETTLE);
        // `stop` returns the whole utterance; keep only what `take_new` had not delivered.
        if rest.len() > audio.len() {
            audio.extend_from_slice(&rest[audio.len()..]);
        }
        if audio.len() > last_cut {
            self.jobs
                .send(Job::Segment { generation, index: sent, audio: audio[last_cut..].to_vec() })
                .map_err(|_| PipelineError::WorkerGone)?;
            sent += 1;
        }
        let mut parts = vec![String::new(); sent];
        let mut received = 0;
        while received < sent {
            let d = self.done.recv().map_err(|_| PipelineError::WorkerGone)?;
            if d.generation == generation {
                parts[d.index] = d.text;
                received += 1;
            }
        }
        let asr_ms = key_up.elapsed().as_secs_f64() * 1e3;
        let raw = parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");

        let gate = if raw.is_empty() { None } else { gate::reason(&raw, &self.cfg.dictionary) };
        let (text, cleanup_ms) = match (&self.llm, gate) {
            (Some(server), Some(_)) => {
                let started = Instant::now();
                let params = ChatParams { max_tokens: (raw.len() as u32 / 2).max(64), ..ChatParams::default() };
                let out = server.chat(&cleanup::messages(&raw, &self.cfg.dictionary), &params)?;
                (cleanup::tidy_output(&out.text), started.elapsed().as_secs_f64() * 1e3)
            }
            _ => (raw.clone(), 0.0),
        };

        let inserted = match (&target, text.is_empty()) {
            (_, true) => Err("nothing was heard".to_owned()),
            (None, false) => Err("no window had the focus".to_owned()),
            (Some(w), false) if do_insert => {
                insert::paste(w, &text, PasteOptions::default()).map_err(|e| e.to_string())
            }
            (Some(_), false) => Err("insertion off (replay)".to_owned()),
        };
        let total_ms = key_up.elapsed().as_secs_f64() * 1e3;
        Ok(Some(Dictation {
            raw,
            text,
            target,
            audio_s: audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
            segments: sent,
            gate,
            inserted,
            asr_ms,
            cleanup_ms,
            total_ms,
        }))
    }
}
