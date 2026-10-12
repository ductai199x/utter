//! The dictation pipeline (DESIGN.md section 4): hotkey -> microphone -> segments cut at
//! pauses and transcribed while the user speaks -> the last segment at key up -> the cleanup
//! gate -> cleanup -> insertion into the window that had the focus at key down.
//!
//! Threads: the hotkey's (utter-platform), the microphone's (utter-audio), a speech worker
//! that owns the recognizer, the cleanup model's server process, and the pipeline loop, which
//! polls the microphone every [`POLL`] while listening. Every dictation's stage times are
//! measured from key up, the moment the user starts waiting.

mod clean;
pub mod gate;
pub mod history;
pub mod log;
pub mod settings;

pub use settings::Settings;

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use utter_asr::boost::BoostConfig;
use utter_asr::vad::{FRAME, Segmenter, SegmenterConfig, SileroVad};
use utter_asr::{ParakeetTdt, SpeechModel};
use utter_audio::{MicConfig, Microphone};
use utter_llm::cleanup::{self, Tier};
use utter_llm::{ChatParams, LlamaServer};
use utter_platform::focus::{self, Window};
use utter_platform::hotkey::{self, Action, HotkeyConfig};
use utter_platform::insert::{self, InsertError};
use utter_platform::shortcut::{self, Shortcut};
use utter_runtime::gpu::GpuWarmer;

use crate::clean::Cleaner;
use crate::history::History;
use crate::log::{DictationLog, Entry, Finished, LogConfig, Timings};
use crate::settings::{KeepHistory, PasteOverride};

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
    /// The user's terms: boosted in recognition and given to cleanup to spell.
    pub dictionary: Vec<String>,
    /// Spoken lists become bullets (false: "Lists: off" in every cleanup request).
    pub lists: bool,
    pub mic: MicConfig,
    pub hotkey: HotkeyConfig,
    /// Skip the cleanup model entirely (raw transcripts).
    pub no_cleanup: bool,
    /// Clean every dictation (true), or only those the gate flags (false).
    pub always_clean: bool,
    /// The cleanup system prompt (None: the product's, prompts/cleanup.md).
    pub prompt: Option<String>,
    pub log: LogConfig,
    /// The person's paste rules over the platform's defaults (docs/ux/insertion.md).
    pub paste: Vec<PasteOverride>,
    pub keep_history: KeepHistory,
    /// A recording (16 kHz mono) each dictation hears instead of the microphone, at real-time
    /// pace, ending the dictation when it runs out: development builds' end-to-end tests,
    /// with no sound played (the app's UTTER_TEST_AUDIO).
    pub test_audio: Option<std::sync::Arc<Vec<f32>>>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            tier: Tier::Quality,
            dictionary: Vec::new(),
            lists: true,
            mic: MicConfig::default(),
            hotkey: HotkeyConfig::default(),
            no_cleanup: false,
            always_clean: true,
            prompt: None,
            log: LogConfig::default(),
            paste: Vec::new(),
            keep_history: KeepHistory::Forever,
            test_audio: None,
        }
    }
}

/// The speech worker's two ends: where jobs go, where results come back.
type Speech = (Sender<Job>, Receiver<Done>);

/// What a model change started: the cleaner, and a new speech worker when the tier runs the
/// speech model elsewhere; or why it could not start.
type ModelLoad = Result<(Option<Cleaner>, Option<Speech>), String>;

/// Which speech build a tier runs: the 8-bit one for both. It transcribes the same as float16
/// and takes 0.66 GB less GPU memory, for about 5 ms more a clip (Tai, 2026-10-11: cut memory
/// as far as quality allows; small latency costs are fine).
pub fn speech_for(_tier: Tier) -> SpeechModel {
    SpeechModel::W8
}

/// GPU memory a tier needs free to run, speech and cleanup together: the peak measured end to
/// end (docs/eval-results.md, 2026-10-11: 4,572 and 3,085 MiB), rounded up.
pub fn gpu_need(tier: Tier) -> u64 {
    match tier {
        Tier::Quality => 4_800_000_000,
        Tier::Light => 3_250_000_000,
        Tier::Reference => 18_000_000_000,
    }
}

/// The smallest graphics card a tier is offered on: what it needs, and about 1 GB more for
/// the desktop and other apps.
pub fn card_need(tier: Tier) -> u64 {
    gpu_need(tier) + 1_000_000_000
}

/// The runtime components a tier needs (crates/utter-runtime/components.json): onnxruntime,
/// the CUDA runtime and cuDNN that llama-server and the speech model use, voice detection,
/// llama-server, then the tier's two models.
pub fn components_for(tier: Tier) -> Vec<&'static str> {
    vec![
        "onnxruntime-gpu",
        "cuda-runtime",
        "cudnn",
        "silero-vad",
        "llama-cpp",
        speech_for(tier).component(),
        tier.component(),
    ]
}

/// Loads and warms the speech model, boosting `dictionary`, on its own worker thread.
fn start_speech(model: SpeechModel, dictionary: &[String]) -> Result<Speech, PipelineError> {
    let mut asr = ParakeetTdt::load(&model.installed_dir()?, model.options())?;
    if !dictionary.is_empty() {
        asr.set_phrases(dictionary, BoostConfig::default());
    }
    for i in 0..3 {
        let t = Instant::now();
        asr.transcribe(&vec![0.0f32; 16_000])?;
        tracing::debug!(pass = i, ms = t.elapsed().as_secs_f64() * 1e3, "speech warm-up");
    }
    let (job_tx, job_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("utter-speech".into())
        .spawn(move || speech_worker(asr, job_rx, done_tx))
        .map_err(|_| PipelineError::WorkerGone)?;
    Ok((job_tx, done_rx))
}

/// How long the paste waits for what is before the cursor: it was asked at key down, so it is
/// almost always there already; an app slower than this gets no space.
const CARET_WAIT: Duration = Duration::from_millis(20);
/// The same for paste last, which asks only when its keys are pressed.
const CARET_WAIT_COMMAND: Duration = Duration::from_millis(150);

/// `text` as it goes in after `before` (the characters before the cursor, `None` when the
/// app does not say): with a space in front when it continues a word or a sentence, the way
/// one types it (docs/ux/insertion.md, "Spacing").
fn spaced(before: Option<&str>, text: &str) -> String {
    if needs_space(before, text) { format!(" {text}") } else { text.to_owned() }
}

fn needs_space(before: Option<&str>, text: &str) -> bool {
    let Some(before) = before else { return false };
    let mut prev = before.chars().rev();
    let (Some(last), Some(first)) = (prev.next(), text.chars().next()) else { return false };
    if last.is_whitespace() || first.is_whitespace() {
        return false;
    }
    // After an opening bracket, a slash or a dash joining words: no space.
    if matches!(last, '(' | '[' | '{' | '\u{201C}' | '\u{2018}' | '/' | '-' | '\u{2013}' | '\u{2014}' | '@' | '#') {
        return false;
    }
    // A straight quote opens when what is before it is a space or nothing.
    if matches!(last, '"' | '\'') && prev.next().is_none_or(char::is_whitespace) {
        return false;
    }
    // Text that starts by closing or punctuating attaches to what is before.
    !matches!(first, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '\u{201D}' | '\u{2019}' | '%')
}

/// How often the idle pipeline looks for new settings.
const SETTINGS_POLL: Duration = Duration::from_millis(200);

/// The context lines every cleanup request carries, from the settings.
fn cleanup_context(cfg: &PipelineConfig) -> cleanup::Context {
    cleanup::Context { lists_off: !cfg.lists, ..cleanup::Context::default() }
}

/// Starts the cleanup model's server and warms it (`None` with cleanup off).
fn start_cleaner(cfg: &PipelineConfig) -> Result<Option<Cleaner>, PipelineError> {
    if cfg.no_cleanup {
        return Ok(None);
    }
    let server = LlamaServer::start(&cleanup::server_options(cfg.tier)?)?;
    let system = cfg.prompt.clone().unwrap_or_else(|| cfg.tier.system_prompt().to_owned());
    // The first request after loading is slow; pay for it now.
    let ctx = cleanup_context(cfg);
    let warm = "Um, this is a warm-up, no, a second warm-up.";
    server.chat(&cleanup::messages_with(&system, warm, &cfg.dictionary, &ctx), &ChatParams::default())?;
    Ok(Some(Cleaner::start(server, system, cfg.dictionary.clone(), ctx)))
}

/// The longest the paste waits for the final cleanup before inserting the raw text instead.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

/// What happened in one dictation, with times measured from key up.
#[derive(Debug, Clone)]
pub struct Dictation {
    pub raw: String,
    pub text: String,
    pub target: Option<Window>,
    pub audio_s: f64,
    pub segments: usize,
    pub gate: Option<&'static str>,
    pub inserted: Result<(), NotInserted>,
    /// It did not go in and was left on the clipboard instead.
    pub copied: bool,
    /// Key up -> every segment transcribed.
    pub asr_ms: f64,
    /// Cleanup request time (0 when the gate skipped it).
    pub cleanup_ms: f64,
    /// The paste (or the copy instead) alone, from the text being ready.
    pub insert_ms: f64,
    /// Key up -> paste sent.
    pub total_ms: f64,
}

/// What the pipeline tells the UI as a dictation goes.
#[derive(Debug, Clone)]
pub enum Event {
    /// Listening began (or became hands-free).
    /// `target` is the window the text will go to (its handle), for placing the UI.
    Listening {
        hands_free: bool,
        target: Option<isize>,
    },
    /// The words so far: `committed` is the dictation cleaned up to the last pause (the text
    /// as it will be typed, though a later correction can still change it); `preview` is what
    /// was heard after that, not cleaned yet (provisional).
    Words {
        committed: String,
        preview: String,
    },
    /// Key up: transcribing the rest, cleaning up, pasting.
    Finishing,
    /// The text being pasted, exactly: the Bar shows it as the paste happens.
    Final {
        text: String,
    },
    Done(Box<Dictation>),
    Cancelled,
    /// A settings change took effect (`error: None`) or could not (the old value stays).
    /// `what` names the setting: "dictionary", "lists", "microphone" or "model".
    Applied {
        what: &'static str,
        error: Option<String>,
    },
    /// A new cleanup model is loading; the current one serves until [`Event::Applied`].
    ModelLoading,
    /// The microphone changed: its level meter replaces the old one's.
    Microphone(utter_audio::LevelMeter),
    /// A word for the Bar outside a dictation: what paste or copy last did ("Pasted",
    /// "Copied: press Ctrl+V to paste"). `ok`: it did what was asked.
    Notice {
        text: String,
        ok: bool,
    },
}

/// Why a dictation did not go in (docs/ux/insertion.md, "When it cannot go in").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotInserted {
    /// Nothing was heard.
    Nothing,
    NoWindow,
    /// The focus moved to another window before the text was ready.
    FocusMoved,
    /// The app runs as administrator: the OS drops keys Utter sends it.
    Refused,
    Failed(String),
    /// A replayed recording (the bench): insertion is off.
    Replay,
}

impl NotInserted {
    /// The reason as History stores it.
    pub fn code(&self) -> &'static str {
        match self {
            NotInserted::Nothing => "nothing",
            NotInserted::NoWindow => "no-window",
            NotInserted::FocusMoved => "focus-moved",
            NotInserted::Refused => "refused",
            NotInserted::Failed(_) => "failed",
            NotInserted::Replay => "replay",
        }
    }

    /// Whether the text should be left on the clipboard instead.
    fn copy_instead(&self) -> bool {
        matches!(self, NotInserted::NoWindow | NotInserted::FocusMoved | NotInserted::Refused | NotInserted::Failed(_))
    }

    /// What the Bar says (`copied`: the text is on the clipboard).
    pub fn message(&self, copied: bool) -> String {
        let keys = insert::primary_keys();
        match (self, copied) {
            (NotInserted::Nothing, _) => "Didn't catch that".to_owned(),
            (NotInserted::Refused, true) => format!("Copied: this app runs as administrator. Press {keys}"),
            (_, true) => format!("Copied: press {keys} to paste"),
            (_, false) => "Couldn't paste or copy it: it is in History".to_owned(),
        }
    }
}

impl std::fmt::Display for NotInserted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotInserted::Nothing => write!(f, "nothing was heard"),
            NotInserted::NoWindow => write!(f, "no window had the focus"),
            NotInserted::FocusMoved => write!(f, "the focus moved to another window"),
            NotInserted::Refused => write!(f, "the app runs as administrator"),
            NotInserted::Failed(e) => write!(f, "{e}"),
            NotInserted::Replay => write!(f, "insertion off (replay)"),
        }
    }
}

impl From<InsertError> for NotInserted {
    fn from(e: InsertError) -> Self {
        match e {
            InsertError::FocusChanged(_) => NotInserted::FocusMoved,
            InsertError::Refused(_) => NotInserted::Refused,
            other => NotInserted::Failed(other.to_string()),
        }
    }
}

/// What the person can ask for outside a dictation: the tray items and their shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PasteLast,
    CopyLast,
}

/// Everything the idle pipeline waits for, on one channel: the hotkey, the shortcuts, the tray.
enum Input {
    Key(Action, Instant),
    Command(Command),
}

/// Where the app sends [`Command`]s (the tray menu).
#[derive(Clone)]
pub struct Commands(Sender<Input>);

impl Commands {
    pub fn send(&self, c: Command) {
        let _ = self.0.send(Input::Command(c));
    }
}

/// How long paste last waits for the shortcut's keys to be let go.
const MODIFIER_WAIT: Duration = Duration::from_secs(1);
/// How often History's retention is applied while Utter runs.
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

enum Job {
    /// From now on, boost these phrases (the user's dictionary).
    Phrases(Vec<String>),
    /// A recognizer pass on a short silent clip, only to keep the GPU busy.
    KeepAlive,
    /// The segment being spoken, transcribed so far, for live words.
    Preview {
        generation: u64,
        audio: Vec<f32>,
    },
    Segment {
        generation: u64,
        index: usize,
        audio: Vec<f32>,
    },
}

enum DoneKind {
    Segment(usize),
    Preview,
}

struct Done {
    generation: u64,
    kind: DoneKind,
    text: String,
    audio_s: f64,
    ms: f64,
}

impl Done {
    fn entry(&self) -> Entry {
        let (pass, index) = match self.kind {
            DoneKind::Segment(i) => ("segment", Some(i)),
            DoneKind::Preview => ("preview", None),
        };
        Entry::Recognized { pass, index, audio_s: self.audio_s, ms: self.ms, text: self.text.clone() }
    }
}

/// What History keeps of a dictation (its id and time are given as it is added).
fn history_entry(d: &Dictation) -> history::Entry {
    let app = d.target.as_ref().map(|w| w.app.clone()).unwrap_or_default();
    history::Entry {
        id: 0,
        at: 0,
        app_exe: app.exe,
        app_name: app.name,
        text: d.text.clone(),
        heard: d.raw.clone(),
        inserted: d.inserted.is_ok(),
        not_inserted: d.inserted.as_ref().err().map(|e| e.code().to_owned()),
        words: d.text.split_whitespace().count(),
        speech_s: d.audio_s,
        ready_ms: d.total_ms,
        favorite: false,
    }
}

/// The next hotkey action within `timeout`; commands that arrive during a dictation are
/// dropped (paste last while dictating has nothing sensible to do).
fn next_key(input: &Receiver<Input>, timeout: Duration) -> Option<(Action, Instant)> {
    let deadline = Instant::now() + timeout;
    loop {
        match input.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Input::Key(action, at)) => return Some((action, at)),
            Ok(Input::Command(c)) => tracing::debug!("{c:?} during a dictation: ignored"),
            Err(_) => return None,
        }
    }
}

fn log_target(target: &Option<Window>) -> Option<(String, u32)> {
    target.as_ref().map(|w| (w.title.clone(), w.process_id))
}

fn transcribe_text(asr: &mut ParakeetTdt, audio: &[f32]) -> String {
    match asr.transcribe(audio) {
        Ok(t) => t.text,
        Err(utter_asr::AsrError::TooShort { .. }) => String::new(),
        Err(e) => {
            tracing::warn!("transcribe: {e}");
            String::new()
        }
    }
}

/// Owns the recognizer; runs jobs in order.
fn speech_worker(mut asr: ParakeetTdt, jobs: Receiver<Job>, done: Sender<Done>) {
    let silence = vec![0.0f32; 8_000];
    while let Ok(mut job) = jobs.recv() {
        // A keep-alive or preview with work queued behind it is stale: skip it. It would only
        // delay the segment, possibly the last one, which the user is waiting for, and a newer
        // preview supersedes an older one.
        while matches!(job, Job::KeepAlive | Job::Preview { .. }) {
            match jobs.try_recv() {
                Ok(next) => job = next,
                Err(_) => break,
            }
        }
        let started = Instant::now();
        let (generation, kind, audio) = match job {
            Job::KeepAlive => {
                let _ = asr.transcribe(&silence);
                continue;
            }
            Job::Phrases(phrases) => {
                asr.set_phrases(&phrases, BoostConfig::default());
                continue;
            }
            Job::Preview { generation, audio } => (generation, DoneKind::Preview, audio),
            Job::Segment { generation, index, audio } => (generation, DoneKind::Segment(index), audio),
        };
        let text = transcribe_text(&mut asr, &audio);
        let audio_s = audio.len() as f64 / utter_audio::SAMPLE_RATE as f64;
        let ms = started.elapsed().as_secs_f64() * 1e3;
        if done.send(Done { generation, kind, text, audio_s, ms }).is_err() {
            break;
        }
    }
}

/// How often the live words are refreshed while speaking; these passes also keep the GPU busy.
const PREVIEW_EVERY: Duration = Duration::from_millis(300);
/// The least unfinished speech worth a preview pass (0.4 s).
const PREVIEW_MIN_SAMPLES: usize = 6_400;

fn join_parts(parts: &[String]) -> String {
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
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

/// Development builds only: with UTTER_TEST_AUDIO naming a 16 kHz mono WAV, every dictation
/// hears that recording instead of the microphone, and an injected Right Ctrl starts one, so
/// the whole app (insertion, History, the Bar) runs end to end with no sound played. Never in
/// an installed copy: there the variable is ignored.
pub fn apply_test_audio(cfg: &mut PipelineConfig) {
    if !utter_core::is_development_build() {
        return;
    }
    let Some(path) = std::env::var_os("UTTER_TEST_AUDIO") else { return };
    let path = std::path::PathBuf::from(path);
    match read_wav_16k_mono(&path) {
        Ok(audio) => {
            tracing::warn!(
                path = %path.display(),
                seconds = audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
                "test audio: dictations hear this file, not the microphone"
            );
            cfg.test_audio = Some(std::sync::Arc::new(audio));
            cfg.hotkey.accept_injected = true;
        }
        Err(e) => tracing::error!("UTTER_TEST_AUDIO {}: {e}", path.display()),
    }
}

fn read_wav_16k_mono(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.sample_rate != utter_audio::SAMPLE_RATE || spec.channels != 1 {
        return Err(format!("{} Hz, {} channels; expected 16 kHz mono", spec.sample_rate, spec.channels));
    }
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => {
            reader.samples::<i16>().map(|s| s.map(|s| s as f32 / 32768.0).map_err(|e| e.to_string())).collect()
        }
        (hound::SampleFormat::Float, _) => reader.samples::<f32>().map(|s| s.map_err(|e| e.to_string())).collect(),
        (_, bits) => Err(format!("{bits}-bit integer samples; expected 16-bit or float")),
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
    /// Settings changes, applied between dictations ([`Pipeline::settings_sender`]).
    settings_tx: Sender<Settings>,
    settings_rx: Receiver<Settings>,
    /// A cleanup model loading for a model change, with the settings that asked for it.
    loading: Option<(Settings, Receiver<ModelLoad>)>,
    input_tx: Sender<Input>,
    input_rx: Receiver<Input>,
    /// When History's retention was last applied.
    pruned: Option<Instant>,
}

/// Everything but the microphone: voice activity detection, the speech worker, the cleanup
/// model. Separate so a dictation can borrow the microphone and the engine at once.
struct Engine {
    cfg: PipelineConfig,
    vad: SileroVad,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    cleaner: Option<Cleaner>,
    generation: u64,
    /// Holds the GPU at full clocks from key down to key up (see utter_runtime::gpu).
    gpu: GpuWarmer,
    /// The newest dictation's text, for paste last when History is off.
    last_text: Option<String>,
}

impl Pipeline {
    /// Loads everything up front (models, server, microphone) and warms it, so the first
    /// dictation is as fast as the rest.
    pub fn start(cfg: PipelineConfig) -> Result<Self, PipelineError> {
        Self::start_reporting(cfg, |_| {})
    }

    /// [`Pipeline::start`], saying which stage is loading as it goes (for the splash,
    /// docs/ux/splash.md): "speech", "cleanup", then "microphone".
    pub fn start_reporting(cfg: PipelineConfig, mut stage: impl FnMut(&'static str)) -> Result<Self, PipelineError> {
        // Everything the loaders will read, read ahead in parallel: a cold start otherwise
        // waits on their piecemeal reads (docs/eval-results.md, 2026-10-12).
        let files = components_for(cfg.tier)
            .into_iter()
            .filter_map(|id| utter_runtime::component(id).ok())
            .flat_map(|c| c.files_on_disk())
            .collect();
        utter_runtime::read_ahead(files);
        // The cleanup server is a process of its own: it loads while the speech model does,
        // so starting takes the longer of the two, not their sum.
        let cleaner_cfg = cfg.clone();
        let cleaner = std::thread::Builder::new()
            .name("utter-cleanup-start".into())
            .spawn(move || start_cleaner(&cleaner_cfg))
            .map_err(|_| PipelineError::WorkerGone)?;
        // The caret reader sets itself up now, not at the first dictation.
        utter_platform::caret::warm();
        stage("speech");
        let (job_tx, done_rx) = start_speech(speech_for(cfg.tier), &cfg.dictionary)?;
        stage("cleanup");
        // Voice detection and the microphone open while the cleanup server finishes.
        let t = Instant::now();
        let vad = SileroVad::load_installed()?;
        let vad_ms = t.elapsed().as_secs_f64() * 1e3;
        // A saved microphone that is not connected is no reason not to start: the system
        // default stands in (the Home window shows the saved one as not connected).
        let mic = match Microphone::open(&cfg.mic) {
            Err(utter_audio::AudioError::NoDevice(_)) if cfg.mic.device.is_some() => {
                tracing::warn!(device = ?cfg.mic.device, "saved microphone not connected; using the default");
                Microphone::open(&MicConfig { device: None, ..cfg.mic.clone() })?
            }
            other => other?,
        };
        let mic_ms = t.elapsed().as_secs_f64() * 1e3 - vad_ms;
        let t = Instant::now();
        let cleaner = cleaner.join().map_err(|_| PipelineError::WorkerGone)??;
        tracing::info!(vad_ms, mic_ms, cleanup_wait_ms = t.elapsed().as_secs_f64() * 1e3, "started");
        stage("microphone");
        let (settings_tx, settings_rx) = mpsc::channel();
        let (input_tx, input_rx) = mpsc::channel();
        Ok(Self {
            mic,
            engine: Engine {
                cfg,
                vad,
                jobs: job_tx,
                done: done_rx,
                cleaner,
                generation: 0,
                gpu: GpuWarmer::new(),
                last_text: None,
            },
            settings_tx,
            settings_rx,
            loading: None,
            input_tx,
            input_rx,
            pruned: None,
        })
    }

    pub fn microphone(&self) -> &Microphone {
        &self.mic
    }

    /// Where to send new settings: they apply between dictations, the newest winning
    /// (docs/ux/home.md, "Saving and applying").
    pub fn settings_sender(&self) -> Sender<Settings> {
        self.settings_tx.clone()
    }

    /// Where to send [`Command`]s: paste or copy the last dictation (the tray menu).
    pub fn commands(&self) -> Commands {
        Commands(self.input_tx.clone())
    }

    /// Runs dictations until the hotkey listener ends; `on_event` sees everything that
    /// happens, from the key going down to the paste. Between dictations it applies settings,
    /// History's retention, and paste or copy last.
    pub fn run(&mut self, mut on_event: impl FnMut(Event)) -> Result<(), PipelineError> {
        let listener = hotkey::listen(self.engine.cfg.hotkey)?;
        let keys = self.input_tx.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::Builder::new()
            .name("utter-keys".into())
            .spawn(move || {
                // Owns the listener; dropping it stops listening. Only one listener can exist, so
                // it must be gone by the time run returns (a Retry starts a new pipeline).
                while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some((action, at)) = listener.recv_timeout(Duration::from_millis(100))
                        && keys.send(Input::Key(action, at)).is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| hotkey::HotkeyError(e.to_string()))?;
        // However run ends (an error included), the key thread stops and the listener goes with it.
        struct StopKeys(std::sync::Arc<std::sync::atomic::AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for StopKeys {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(t) = self.1.take() {
                    let _ = t.join();
                }
            }
        }
        let _keys = StopKeys(stop, Some(thread));
        let commands = self.input_tx.clone();
        let _shortcuts = match shortcut::register(move |s| {
            let c = match s {
                Shortcut::PasteLast => Command::PasteLast,
                Shortcut::CopyLast => Command::CopyLast,
            };
            let _ = commands.send(Input::Command(c));
        }) {
            Ok(s) => {
                for t in &s.taken {
                    tracing::warn!("{} is taken by another program; the tray item still works", t.keys());
                }
                Some(s)
            }
            Err(e) => {
                tracing::warn!("{e}");
                None
            }
        };
        loop {
            self.apply_settings(&mut on_event);
            self.prune_history(false);
            match self.input_rx.recv_timeout(SETTINGS_POLL) {
                Ok(Input::Key(Action::Start, _)) => {
                    let looked = Instant::now();
                    let target = focus::foreground();
                    tracing::debug!(ms = looked.elapsed().as_secs_f64() * 1e3, "key down: the focused app");
                    let test_audio = self.engine.cfg.test_audio.clone();
                    let mut mic = MicSource(&self.mic);
                    let mut replay;
                    let source: &mut dyn AudioSource = match &test_audio {
                        Some(audio) => {
                            replay = ReplaySource::new(audio);
                            &mut replay
                        }
                        None => &mut mic,
                    };
                    let input = &self.input_rx;
                    self.engine.dictate(source, |t| next_key(input, t), target, true, &mut on_event)?;
                }
                Ok(Input::Command(c)) => self.engine.command(c, &mut on_event),
                Ok(Input::Key(..)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }

    /// Applies History's retention when Utter starts, every hour, and when it changes (`now`).
    fn prune_history(&mut self, now: bool) {
        if !now && self.pruned.is_some_and(|t| t.elapsed() < PRUNE_EVERY) {
            return;
        }
        self.pruned = Some(Instant::now());
        match History::global().prune(self.engine.cfg.keep_history.max_age()) {
            Ok(0) => {}
            Ok(n) => tracing::info!("history: {n} entries past their keep time deleted"),
            Err(e) => tracing::warn!("history: {e}"),
        }
    }

    /// Applies the newest settings sent, and swaps in a cleanup model that finished loading.
    fn apply_settings(&mut self, on_event: &mut impl FnMut(Event)) {
        if let Some((wanted, rx)) = &self.loading {
            match rx.try_recv() {
                Ok(result) => {
                    let error = match result {
                        Ok((cleaner, speech)) => {
                            self.engine.cleaner = cleaner;
                            if let Some((jobs, done)) = speech {
                                // The old worker ends when its job channel closes. The
                                // dictionary may have changed while the new one loaded.
                                let _ = jobs.send(Job::Phrases(self.engine.cfg.dictionary.clone()));
                                self.engine.jobs = jobs;
                                self.engine.done = done;
                            }
                            self.engine.cfg.tier = wanted.model.into();
                            None
                        }
                        Err(e) => Some(e),
                    };
                    self.loading = None;
                    on_event(Event::Applied { what: "model", error });
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                    on_event(Event::Applied { what: "model", error: Some("the model loader stopped".into()) });
                }
            }
        }
        let Some(s) = self.settings_rx.try_iter().last() else { return };
        let mut wanted = self.engine.cfg.clone();
        s.apply(&mut wanted);
        self.engine.cfg.paste = wanted.paste;
        self.engine.cfg.log = wanted.log;
        if wanted.keep_history != self.engine.cfg.keep_history {
            self.engine.cfg.keep_history = wanted.keep_history;
            self.prune_history(true);
        }
        let cfg = &mut self.engine.cfg;
        if s.dictionary != cfg.dictionary || s.lists != cfg.lists {
            let what = if s.dictionary != cfg.dictionary { "dictionary" } else { "lists" };
            if s.dictionary != cfg.dictionary {
                let _ = self.engine.jobs.send(Job::Phrases(s.dictionary.clone()));
            }
            cfg.dictionary = s.dictionary.clone();
            cfg.lists = s.lists;
            if let Some(cleaner) = &self.engine.cleaner {
                cleaner.configure(cfg.dictionary.clone(), cleanup_context(cfg));
            }
            on_event(Event::Applied { what, error: None });
        }
        let device = (!s.microphone.is_empty()).then(|| s.microphone.clone());
        if device != cfg.mic.device {
            let mic_cfg = MicConfig { device: device.clone(), ..cfg.mic.clone() };
            match Microphone::open(&mic_cfg) {
                Ok(mic) => {
                    cfg.mic = mic_cfg;
                    self.mic = mic;
                    on_event(Event::Microphone(self.mic.level_meter()));
                    on_event(Event::Applied { what: "microphone", error: None });
                }
                Err(e) => on_event(Event::Applied { what: "microphone", error: Some(e.to_string()) }),
            }
        }
        let tier: Tier = s.model.into();
        let loading_tier = self.loading.as_ref().map(|(w, _)| Tier::from(w.model));
        if tier == cfg.tier && loading_tier.is_some() {
            // Changed back while the other model loads: it is dropped (and its server ended)
            // when it arrives to a closed channel.
            self.loading = None;
            on_event(Event::Applied { what: "model", error: None });
        } else if !cfg.no_cleanup && tier != cfg.tier && loading_tier != Some(tier) {
            // The new model loads beside the old one, which serves until it is ready.
            let mut next = cfg.clone();
            next.tier = tier;
            next.prompt = None;
            let new_speech = (speech_for(tier) != speech_for(cfg.tier)).then_some(speech_for(tier));
            let (tx, rx) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("utter-model-load".into())
                .spawn(move || {
                    let load = || -> Result<_, PipelineError> {
                        let speech = new_speech.map(|m| start_speech(m, &next.dictionary)).transpose()?;
                        Ok((start_cleaner(&next)?, speech))
                    };
                    let _ = tx.send(load().map_err(|e| e.to_string()));
                })
                .is_ok();
            if spawned {
                self.loading = Some((s, rx));
                on_event(Event::ModelLoading);
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
            &mut |_| {},
        )?;
        Ok(d.expect("a replay is never cancelled"))
    }
}

impl Engine {
    /// Paste or copy the last dictation (docs/ux/insertion.md).
    fn command(&mut self, c: Command, on_event: &mut impl FnMut(Event)) {
        let text = self.last_text.clone().or_else(|| History::global().last_text());
        let Some(text) = text else {
            on_event(Event::Notice { text: "Nothing to paste yet".into(), ok: false });
            return;
        };
        let copy = |on_event: &mut dyn FnMut(Event), why: Option<NotInserted>| {
            let copied = insert::copy(&text).inspect_err(|e| tracing::warn!("copy: {e}")).is_ok();
            let text = match (&why, copied) {
                (None, true) => "Copied".to_owned(),
                (Some(why), _) => why.message(copied),
                (None, false) => "Couldn't copy it".to_owned(),
            };
            on_event(Event::Notice { text, ok: copied && why.is_none() });
        };
        match c {
            Command::CopyLast => copy(on_event, None),
            Command::PasteLast => {
                // The shortcut fires with its keys down; paste keys sent now would reach the
                // app with them.
                shortcut::wait_for_modifiers_released(MODIFIER_WAIT);
                // From the tray, the taskbar has the focus: nothing to paste into.
                let target = focus::foreground()
                    .filter(|w| !(w.title.is_empty() && w.app.exe.eq_ignore_ascii_case("explorer.exe")));
                let Some(w) = target else { return copy(on_event, Some(NotInserted::NoWindow)) };
                let before = utter_platform::caret::before_cursor().get(CARET_WAIT_COMMAND);
                let text = spaced(before.as_deref(), &text);
                match insert::paste(&w, &text, settings::paste_rule(&self.cfg.paste, &w.app.exe)) {
                    Ok(()) => on_event(Event::Notice { text: "Pasted".into(), ok: true }),
                    Err(e) => copy(on_event, Some(e.into())),
                }
            }
        }
    }

    /// Whether this dictation's text goes through the cleanup model.
    fn should_clean(&self, raw: &str) -> bool {
        self.cfg.always_clean || gate::reason(raw, &self.cfg.dictionary).is_some()
    }

    /// What the dictation ran with, for its log record.
    fn log_config(&self) -> serde_json::Value {
        let version = |id: &str| utter_runtime::component(id).map(|c| c.version.clone()).ok();
        serde_json::json!({
            "speech_model": {
                "id": speech_for(self.cfg.tier).component(),
                "version": version(speech_for(self.cfg.tier).component())
            },
            "cleanup_model": if self.cfg.no_cleanup {
                serde_json::Value::Null
            } else {
                serde_json::json!({ "id": self.cfg.tier.component(), "version": version(self.cfg.tier.component()) })
            },
            "dictionary": self.cfg.dictionary,
            "lists": self.cfg.lists,
        })
    }

    /// One dictation, from the Start action: returns `None` if it was cancelled.
    fn dictate(
        &mut self,
        source: &mut dyn AudioSource,
        next_action: impl FnMut(Duration) -> Option<(Action, Instant)>,
        target: Option<Window>,
        do_insert: bool,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Option<Dictation>, PipelineError> {
        let result = self.dictate_inner(source, next_action, target, do_insert, on_event);
        // Whatever happened, the GPU goes back to idling.
        self.gpu.stop();
        result
    }

    fn dictate_inner(
        &mut self,
        source: &mut dyn AudioSource,
        mut next_action: impl FnMut(Duration) -> Option<(Action, Instant)>,
        target: Option<Window>,
        do_insert: bool,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<Option<Dictation>, PipelineError> {
        self.generation += 1;
        let generation = self.generation;
        // What is before the cursor, read while the person speaks: a dictation that continues
        // text gets a space (needs_space).
        let caret = do_insert.then(utter_platform::caret::before_cursor);
        // Live dictations only: a replay (the bench) is not the user's.
        let mut log = DictationLog::start(LogConfig { enabled: self.cfg.log.enabled && do_insert, ..self.cfg.log });
        log.push(Entry::Listening { hands_free: false });
        source.start();
        self.gpu.start();
        let target_handle = target.as_ref().map(|w| w.handle);
        on_event(Event::Listening { hands_free: false, target: target_handle });
        self.vad.reset();
        let mut segmenter = Segmenter::new(SegmenterConfig::default());
        let mut audio: Vec<f32> = Vec::new();
        let mut last_cut = 0usize;
        let mut sent = 0usize;
        let mut parts: Vec<String> = Vec::new();
        let mut received = 0usize;
        let mut preview = String::new();
        let mut last_job = Instant::now();
        let mut last_preview = Instant::now();
        // The newest cleanup while speaking: (the raw text it cleaned, the result), and the raw
        // text of the newest request (so the same text is never sent twice).
        let mut latest: Option<(String, String)> = None;
        let mut requested: Option<String> = None;
        let _ = self.jobs.send(Job::KeepAlive);

        let key_up = loop {
            if source.ended() {
                break Instant::now();
            }
            match next_action(POLL) {
                Some((Action::Finish, at)) => break at,
                Some((Action::Cancel, _)) => {
                    let audio = source.stop(Duration::ZERO);
                    on_event(Event::Cancelled);
                    log.push(Entry::Cancelled);
                    log.finish(Finished {
                        target: log_target(&target),
                        config: self.log_config(),
                        raw: None,
                        text: None,
                        audio,
                        timings: None,
                    });
                    return Ok(None);
                }
                Some((Action::HandsFree, _)) => {
                    log.push(Entry::Listening { hands_free: true });
                    on_event(Event::Listening { hands_free: true, target: target_handle });
                }
                Some((Action::Start, _)) | None => {}
            }
            // Results that came back while speaking: show them.
            let mut words_changed = false;
            let mut segment_done = false;
            while let Ok(d) = self.done.try_recv() {
                if d.generation != generation {
                    continue;
                }
                log.push(d.entry());
                match d.kind {
                    DoneKind::Segment(i) => {
                        parts[i] = d.text;
                        received += 1;
                        segment_done = true;
                        // The preview was of this segment (or earlier audio); the next one
                        // starts fresh.
                        preview.clear();
                    }
                    DoneKind::Preview => preview = d.text,
                }
                words_changed = true;
            }
            let committed_raw = join_parts(&parts);
            if let Some(cleaner) = &self.cleaner {
                // A segment finished: clean the dictation so far, while the user goes on.
                if segment_done
                    && !committed_raw.is_empty()
                    && requested.as_deref() != Some(committed_raw.as_str())
                    && self.should_clean(&committed_raw)
                {
                    cleaner.request(generation, committed_raw.clone());
                    requested = Some(committed_raw.clone());
                }
                while let Some(c) = cleaner.try_recv() {
                    if c.generation != generation {
                        continue;
                    }
                    log.push(c.entry("live"));
                    // A failed cleanup shows the raw text: the words are never lost.
                    let text = c.text.unwrap_or_else(|_| c.raw.clone());
                    latest = Some((c.raw, text));
                    words_changed = true;
                }
            }
            if words_changed {
                let (committed, provisional) = if self.cleaner.is_some() {
                    clean::bar_words(latest.as_ref().map(|(r, t)| (r.as_str(), t.as_str())), &committed_raw, &preview)
                } else {
                    (committed_raw, preview.clone())
                };
                log.push(Entry::Bar { committed: committed.clone(), preview: provisional.clone(), is_final: false });
                on_event(Event::Words { committed, preview: provisional });
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
                    log.push(Entry::Segment {
                        index: sent,
                        start_s: last_cut as f64 / utter_audio::SAMPLE_RATE as f64,
                        end_s: cut_sample as f64 / utter_audio::SAMPLE_RATE as f64,
                    });
                    let segment = audio[last_cut..cut_sample].to_vec();
                    self.jobs
                        .send(Job::Segment { generation, index: sent, audio: segment })
                        .map_err(|_| PipelineError::WorkerGone)?;
                    parts.push(String::new());
                    sent += 1;
                    last_cut = cut_sample;
                    last_job = Instant::now();
                }
            }
            if last_preview.elapsed() >= PREVIEW_EVERY
                && segmenter.has_speech()
                && audio.len() - last_cut >= PREVIEW_MIN_SAMPLES
            {
                let _ = self.jobs.send(Job::Preview { generation, audio: audio[last_cut..].to_vec() });
                last_preview = Instant::now();
                last_job = last_preview;
            } else if last_job.elapsed() >= KEEPALIVE {
                let _ = self.jobs.send(Job::KeepAlive);
                last_job = Instant::now();
            }
        };

        // Key up: the user is waiting from here on.
        self.gpu.stop();
        log.push(Entry::KeyUp);
        on_event(Event::Finishing);
        let rest = source.stop(SETTLE);
        // `stop` returns the whole utterance; keep only what `take_new` had not delivered.
        if rest.len() > audio.len() {
            audio.extend_from_slice(&rest[audio.len()..]);
        }
        if audio.len() > last_cut {
            log.push(Entry::Segment {
                index: sent,
                start_s: last_cut as f64 / utter_audio::SAMPLE_RATE as f64,
                end_s: audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
            });
            self.jobs
                .send(Job::Segment { generation, index: sent, audio: audio[last_cut..].to_vec() })
                .map_err(|_| PipelineError::WorkerGone)?;
            parts.push(String::new());
            sent += 1;
        }
        while received < sent {
            let d = self.done.recv().map_err(|_| PipelineError::WorkerGone)?;
            if d.generation != generation {
                continue;
            }
            log.push(d.entry());
            if let DoneKind::Segment(i) = d.kind {
                parts[i] = d.text;
                received += 1;
            }
        }
        let asr_ms = key_up.elapsed().as_secs_f64() * 1e3;
        let raw = join_parts(&parts);

        // The gate is logged either way; it decides only when not every dictation is cleaned.
        let gate = if raw.is_empty() { None } else { gate::reason(&raw, &self.cfg.dictionary) };
        log.push(Entry::Gate { reason: gate });
        let started = Instant::now();
        let text = match &self.cleaner {
            Some(cleaner) if !raw.is_empty() && self.should_clean(&raw) => {
                match latest {
                    // Nothing was heard since the last pause's cleanup: it is the final text.
                    Some((done_raw, done_text)) if done_raw == raw => {
                        log.push(Entry::Cleanup {
                            when: "reused",
                            input: raw.clone(),
                            output: done_text.clone(),
                            ms: 0.0,
                            error: None,
                        });
                        done_text
                    }
                    _ => {
                        if requested.as_deref() != Some(raw.as_str()) {
                            cleaner.request(generation, raw.clone());
                        }
                        match cleaner.wait_for(generation, &raw, CLEANUP_TIMEOUT) {
                            Some(c) => {
                                log.push(c.entry("final"));
                                // The words are never lost to a cleanup failure: the raw text goes in.
                                c.text.unwrap_or_else(|e| {
                                    tracing::warn!("cleanup failed, inserting the raw text: {e}");
                                    raw.clone()
                                })
                            }
                            None => {
                                tracing::warn!("cleanup took over {CLEANUP_TIMEOUT:?}; inserting the raw text");
                                log.push(Entry::Cleanup {
                                    when: "final",
                                    input: raw.clone(),
                                    output: raw.clone(),
                                    ms: started.elapsed().as_secs_f64() * 1e3,
                                    error: Some("timed out".into()),
                                });
                                raw.clone()
                            }
                        }
                    }
                }
            }
            _ => raw.clone(),
        };
        let cleanup_ms = started.elapsed().as_secs_f64() * 1e3;

        // The Bar shows exactly what is pasted, as it is pasted.
        log.push(Entry::Bar { committed: text.clone(), preview: String::new(), is_final: true });
        let shown = Instant::now();
        on_event(Event::Final { text: text.clone() });
        tracing::debug!(ms = shown.elapsed().as_secs_f64() * 1e3, "the Bar told the final text");
        let insert_start = Instant::now();
        let inserted = match (&target, text.is_empty()) {
            (_, true) => Err(NotInserted::Nothing),
            (_, false) if !do_insert => Err(NotInserted::Replay),
            (None, false) => Err(NotInserted::NoWindow),
            (Some(w), false) => {
                let before = caret.as_ref().and_then(|c| c.get(CARET_WAIT));
                let pasted = spaced(before.as_deref(), &text);
                tracing::debug!(?before, spaced = pasted.len() != text.len(), "before the cursor");
                insert::paste(w, &pasted, settings::paste_rule(&self.cfg.paste, &w.app.exe)).map_err(NotInserted::from)
            }
        };
        // A dictation is never lost: what could not go in is left on the clipboard.
        let copied = match &inserted {
            Err(e) if e.copy_instead() => insert::copy(&text).inspect_err(|c| tracing::warn!("copy: {c}")).is_ok(),
            _ => false,
        };
        let insert_ms = insert_start.elapsed().as_secs_f64() * 1e3;
        let total_ms = key_up.elapsed().as_secs_f64() * 1e3;
        log.push(Entry::Inserted {
            ok: inserted.is_ok(),
            error: inserted.as_ref().err().map(ToString::to_string),
            ms: insert_ms,
        });
        let dictation = Dictation {
            raw,
            text,
            target,
            audio_s: audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
            segments: sent,
            gate,
            inserted,
            copied,
            asr_ms,
            cleanup_ms,
            insert_ms,
            total_ms,
        };
        if do_insert && !dictation.text.trim().is_empty() {
            self.last_text = Some(dictation.text.clone());
            if self.cfg.keep_history != KeepHistory::Off
                && let Err(e) = History::global().add(history_entry(&dictation))
            {
                tracing::warn!("history: {e}");
            }
        }
        on_event(Event::Done(Box::new(dictation.clone())));
        // The audio is not needed past here: it moves into the log, uncopied.
        log.finish(Finished {
            target: log_target(&dictation.target),
            config: self.log_config(),
            raw: Some(dictation.raw.clone()),
            text: Some(dictation.text.clone()),
            audio,
            timings: Some(Timings { asr: asr_ms, cleanup: cleanup_ms, insert: insert_ms, total: total_ms }),
        });
        Ok(Some(dictation))
    }
}

#[cfg(test)]
mod tests {
    use super::needs_space;

    #[test]
    fn a_dictation_after_text_gets_a_space() {
        assert!(needs_space(Some("s."), "There is"));
        assert!(needs_space(Some("rd"), "and more"));
        assert!(needs_space(Some("?\""), "Then"));
        assert!(needs_space(Some("e)"), "next"));
    }

    #[test]
    fn no_space_where_typing_would_not_put_one() {
        // Unknown, the start of the field, after a space or a line break.
        assert!(!needs_space(None, "Hello"));
        assert!(!needs_space(Some(""), "Hello"));
        assert!(!needs_space(Some("o "), "Hello"));
        assert!(!needs_space(Some(".\n"), "Hello"));
        assert!(!needs_space(Some(".\r"), "Hello"));
        // After an opening bracket, quote, slash or joining dash.
        assert!(!needs_space(Some(" ("), "aside"));
        assert!(!needs_space(Some(" \""), "quoted"));
        assert!(!needs_space(Some("\""), "quoted"));
        assert!(!needs_space(Some("d/"), "or"));
        assert!(!needs_space(Some("e-"), "mail"));
        // Text that starts with punctuation, or nothing.
        assert!(!needs_space(Some("rd"), ", and more"));
        assert!(!needs_space(Some("rd"), "."));
        assert!(!needs_space(Some("rd"), ""));
    }
}
