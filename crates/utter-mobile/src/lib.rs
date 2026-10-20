//! Utter's core on Android (docs/android.md, "Architecture"): the dictation session behind
//! the app's few JNI calls ([`jni`]).
//!
//! The app hands over what happens on the bubble and the microphone's audio; everything that
//! decides lives here, on a worker thread of its own: press and tap timing (the desktop
//! hotkey's [`HotkeyMachine`], the bubble acting as the key, a drag off it as Esc), segments
//! and live words, the gate and cleanup ([`utter_pipeline::Session`]). What happens comes back
//! as [`Event`]s the app polls, so the core runs and is tested without a phone
//! (`examples/replay.rs`).

pub mod jni;
mod logcat;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use utter_asr::litert::Accelerator;
use utter_asr::vad::SileroVad;
use utter_asr::{Device, ParakeetOptions, ParakeetTdt};
use utter_llm::cleanup::{self, Tier};
use utter_llm::{LlamaServer, ServerOptions};
use utter_pipeline::log::{DictationLog, Entry, Finished, LogConfig, Timings};
use utter_pipeline::{CleanRule, Recognizer, Session};
use utter_platform::hotkey::{Action, HotkeyConfig, HotkeyMachine, KeyEvent, VK_ESCAPE};

/// How often the worker looks for results while listening (the desktop pipeline's poll).
const POLL: Duration = utter_pipeline::POLL;
/// The bubble, as the hotkey machine's key.
const BUBBLE: u32 = 0x0100;

/// What the app gives at load: where everything is.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Utter's data folder (the app's files folder): logs, and later History and settings.
    pub data_dir: PathBuf,
    /// The app's native library folder: onnxruntime, LiteRT and llama-server are there.
    pub native_dir: PathBuf,
    /// The speech model's folder: encoder.tflite and its .json, decoder.onnx, joint.onnx,
    /// config.json, vocab.txt, mel_filters.f32.
    pub speech_dir: PathBuf,
    /// Silero VAD's ONNX file.
    pub vad: PathBuf,
    /// The encoder's accelerator: the GPU on a phone, the CPU on the emulator.
    #[serde(default)]
    pub cpu_encoder: bool,
    /// The cleanup model's GGUF (None: no cleanup, the raw transcript goes in).
    pub cleanup_model: Option<PathBuf>,
    /// The CPU threads llama-server decodes with (4 measured best on the test phone).
    #[serde(default = "default_threads")]
    pub cleanup_threads: u32,
    #[serde(default)]
    pub dictionary: Vec<String>,
    #[serde(default = "yes")]
    pub lists: bool,
    /// Keep each dictation's record and audio in `<data_dir>/dictations` (the desktop's
    /// default too).
    #[serde(default = "yes")]
    pub logs: bool,
}

fn default_threads() -> u32 {
    4
}

fn yes() -> bool {
    true
}

/// What happens, for the app to show (docs/ux/android.md, "Dictating").
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Listening began: the bubble grows and the pill opens.
    Listening {
        hands_free: bool,
    },
    /// The words so far: cleaned up to the last pause, and heard since (provisional).
    Words {
        committed: String,
        preview: String,
    },
    /// Speech ended: transcribing the rest and cleaning up.
    Finishing,
    /// The text, and what was heard; times from the end of speech. `insert` is the text as it
    /// goes in after what was before the cursor ([`Mobile::before_cursor`]): with a space in
    /// front where typing would put one.
    Final {
        text: String,
        insert: String,
        raw: String,
        asr_ms: f64,
        cleanup_ms: f64,
        total_ms: f64,
        audio_s: f64,
    },
    /// Dropped: a drag off the bubble, or a press too short to be a dictation.
    Cancelled,
    Error {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("{0}")]
    Asr(#[from] utter_asr::AsrError),
    #[error("{0}")]
    Llm(#[from] utter_llm::LlmError),
    #[error("{0}")]
    Pipeline(#[from] utter_pipeline::PipelineError),
    #[error("the core's worker stopped")]
    Gone,
}

enum Input {
    Touch {
        down: bool,
        time_ms: u32,
    },
    /// A drag off the bubble.
    Cancel,
    Audio(Vec<f32>),
    BeforeCursor(String),
    /// The app freed the core: end the worker, and with it the models and llama-server.
    Stop,
}

/// The events not yet taken by the app.
#[derive(Default)]
struct Events {
    queue: Mutex<VecDeque<Event>>,
    ready: Condvar,
}

impl Events {
    fn push(&self, e: Event) {
        tracing::debug!(?e, "event");
        self.queue.lock().expect("events lock").push_back(e);
        self.ready.notify_one();
    }

    fn pop(&self, timeout: Duration) -> Option<Event> {
        let queue = self.queue.lock().expect("events lock");
        let (mut queue, _) = self.ready.wait_timeout_while(queue, timeout, |q| q.is_empty()).expect("events lock");
        queue.pop_front()
    }
}

/// Utter's loaded core: the models, and the worker that runs dictations.
pub struct Mobile {
    input: Sender<Input>,
    worker: Option<std::thread::JoinHandle<()>>,
    events: Arc<Events>,
}

impl Mobile {
    /// Loads the speech model, voice detection and the cleanup model, warms them, and starts
    /// the worker. Takes seconds (the GPU encoder's first load longer): call it off the UI
    /// thread.
    pub fn load(cfg: &Config) -> Result<Self, MobileError> {
        utter_core::set_data_dir(cfg.data_dir.clone());
        utter_asr::runtime::use_library(cfg.native_dir.join("libonnxruntime.so"));
        utter_asr::litert::use_library(cfg.native_dir.join("libLiteRt.so"));
        let started = Instant::now();
        // The cleanup server is a process of its own: it loads while the speech model does.
        let cleanup = cfg.cleanup_model.clone().map(|model| {
            let opts = server_options(model, cfg);
            let (dictionary, ctx) = (cfg.dictionary.clone(), cleanup::Context { lists_off: !cfg.lists });
            std::thread::spawn(move || -> Result<_, MobileError> {
                let server = LlamaServer::start(&opts)?;
                let system = Tier::Light.system_prompt().to_owned();
                Ok(utter_pipeline::session::start_cleaner(server, system, dictionary, ctx)?)
            })
        });
        let accelerator = if cfg.cpu_encoder { Accelerator::Cpu } else { Accelerator::Gpu };
        let opts = ParakeetOptions { encoder: Device::LiteRt(accelerator), decoder: Device::Cpu, ..Default::default() };
        let asr = ParakeetTdt::load(&cfg.speech_dir, opts)?;
        let vad = SileroVad::load(&cfg.vad)?;
        let cleaner = match cleanup {
            Some(handle) => Some(handle.join().map_err(|_| MobileError::Gone)??),
            None => None,
        };
        let rec = Recognizer::start(asr, vad, cleaner, &cfg.dictionary)?;
        tracing::info!(load_ms = started.elapsed().as_secs_f64() * 1e3, "utter-mobile loaded");

        let (input, input_rx) = mpsc::channel();
        let events = Arc::new(Events::default());
        let worker = Worker {
            rec,
            rule: CleanRule { always: true, dictionary: cfg.dictionary.clone() },
            logs: LogConfig { enabled: cfg.logs, audio: cfg.logs },
            log_config: serde_json::json!({
                "speech_model": { "id": "litert", "dir": cfg.speech_dir, "cpu": cfg.cpu_encoder },
                "cleanup_model": cfg.cleanup_model,
                "dictionary": cfg.dictionary,
                "lists": cfg.lists,
            }),
            keys: HotkeyMachine::new(HotkeyConfig { key: BUBBLE, ..Default::default() }),
            events: events.clone(),
            dictation: None,
        };
        let worker = std::thread::Builder::new()
            .name("utter-mobile".into())
            .spawn(move || worker.run(input_rx))
            .map_err(|_| MobileError::Gone)?;
        Ok(Self { input, worker: Some(worker), events })
    }

    /// The bubble went down (`down`) or up, at `time_ms` on the app's uptime clock.
    pub fn touch(&self, down: bool, time_ms: u32) {
        let _ = self.input.send(Input::Touch { down, time_ms });
    }

    /// The finger left the bubble while holding it: drop the dictation.
    pub fn cancel(&self) {
        let _ = self.input.send(Input::Cancel);
    }

    /// Microphone audio, 16 kHz mono, as it arrives.
    /// What is before the cursor in the field being dictated into, read as listening starts
    /// (as the desktop reads it at key down): the final text is spaced after it.
    pub fn before_cursor(&self, text: String) {
        let _ = self.input.send(Input::BeforeCursor(text));
    }

    pub fn push_audio(&self, samples: Vec<f32>) {
        let _ = self.input.send(Input::Audio(samples));
    }

    /// The next event, waiting up to `timeout` for one.
    pub fn next_event(&self, timeout: Duration) -> Option<Event> {
        self.events.pop(timeout)
    }
}

impl Drop for Mobile {
    /// Returns once the worker, the models and llama-server are gone, not some time later.
    fn drop(&mut self) {
        let _ = self.input.send(Input::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// llama-server as the phone runs it (docs/android.md, "The phone cleanup model"):
/// the app's own build on the CPU, flash attention off, the desktop's drafting arguments.
fn server_options(model: PathBuf, cfg: &Config) -> ServerOptions {
    let mut opts = ServerOptions::new(model);
    opts.exe = Some(cfg.native_dir.join("libllama-server.so"));
    opts.gpu_layers = 0;
    opts.flash_attention = false;
    opts.extra_args = cleanup::SERVER_ARGS.iter().map(|s| (*s).to_owned()).collect();
    opts.extra_args.extend(["--threads".to_owned(), cfg.cleanup_threads.to_string()]);
    opts
}

/// One dictation under way.
struct Dictation {
    session: Session,
    log: DictationLog,
    /// What is before the cursor (None: the app has not said).
    before: Option<String>,
}

struct Worker {
    rec: Recognizer,
    rule: CleanRule,
    /// Each dictation's record (utter_pipeline::log), as on the desktop.
    logs: LogConfig,
    /// What the dictations run with, for their records.
    log_config: serde_json::Value,
    keys: HotkeyMachine,
    events: Arc<Events>,
    dictation: Option<Dictation>,
}

impl Worker {
    fn run(mut self, input: Receiver<Input>) {
        loop {
            match input.recv_timeout(POLL) {
                Ok(Input::Touch { down, time_ms }) => {
                    let action = self.keys.on_event(KeyEvent { vk: BUBBLE, down, time_ms, injected: false });
                    self.act(action);
                }
                Ok(Input::Cancel) => {
                    let action =
                        self.keys.on_event(KeyEvent { vk: VK_ESCAPE, down: true, time_ms: 0, injected: false });
                    self.act(action);
                }
                Ok(Input::Audio(samples)) => {
                    if let Some(d) = &mut self.dictation
                        && let Err(e) = d.session.push(&mut self.rec, &samples, &mut d.log)
                    {
                        self.dictation = None;
                        self.events.push(Event::Error { message: e.to_string() });
                    }
                }
                Ok(Input::BeforeCursor(text)) => {
                    if let Some(d) = &mut self.dictation {
                        d.before = Some(text);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Ok(Input::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            }
            if let Some(d) = &mut self.dictation
                && let Some((committed, preview)) = d.session.poll(&self.rec, &mut d.log)
            {
                self.events.push(Event::Words { committed, preview });
            }
        }
    }

    fn act(&mut self, action: Option<Action>) {
        match action {
            Some(action @ (Action::Start | Action::HandsFree)) => {
                let hands_free = action == Action::HandsFree;
                let mut log = DictationLog::start(self.logs);
                log.push(Entry::Listening { hands_free });
                let session = Session::start(&mut self.rec, self.rule.clone());
                self.dictation = Some(Dictation { session, log, before: None });
                self.events.push(Event::Listening { hands_free });
            }
            Some(Action::Finish) => {
                let Some(Dictation { session, mut log, before }) = self.dictation.take() else { return };
                let ended = Instant::now();
                log.push(Entry::KeyUp);
                self.events.push(Event::Finishing);
                match session.finish(&mut self.rec, &[], ended, &mut log) {
                    Ok(heard) => {
                        let total_ms = ended.elapsed().as_secs_f64() * 1e3;
                        self.events.push(Event::Final {
                            audio_s: heard.audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
                            insert: utter_pipeline::spaced(before.as_deref(), &heard.text),
                            text: heard.text.clone(),
                            raw: heard.raw.clone(),
                            asr_ms: heard.asr_ms,
                            cleanup_ms: heard.cleanup_ms,
                            total_ms,
                        });
                        log.finish(Finished {
                            target: None,
                            config: self.log_config.clone(),
                            raw: Some(heard.raw),
                            text: Some(heard.text),
                            audio: heard.audio,
                            timings: Some(Timings {
                                asr: heard.asr_ms,
                                cleanup: heard.cleanup_ms,
                                insert: 0.0,
                                total: total_ms,
                            }),
                        });
                    }
                    Err(e) => self.events.push(Event::Error { message: e.to_string() }),
                }
            }
            Some(Action::Cancel) => {
                let Some(Dictation { session, mut log, .. }) = self.dictation.take() else { return };
                log.push(Entry::Cancelled);
                let audio = session.audio().to_vec();
                let config = self.log_config.clone();
                log.finish(Finished { target: None, config, raw: None, text: None, audio, timings: None });
                self.events.push(Event::Cancelled);
            }
            None => {}
        }
    }
}
