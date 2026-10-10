//! The dictation log: one record per dictation, with every recognizer pass, every state of
//! the Bar, the cleanup request and the result, each timed from key down, plus the audio.
//! It stays on this machine (`<data dir>/dictations/`) and is what we read when a dictation
//! goes wrong (docs/plan-final-text.md, "Logging").

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// What to keep.
#[derive(Debug, Clone, Copy)]
pub struct LogConfig {
    pub enabled: bool,
    /// Also save the audio (16-bit WAV, about 32 KB per second).
    pub audio: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self { enabled: true, audio: true }
    }
}

/// Where the records go.
pub fn log_dir() -> PathBuf {
    utter_core::data_dir().join("dictations")
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Listening {
        hands_free: bool,
    },
    /// A pause cut a segment, sent to the recognizer.
    Segment {
        index: usize,
        start_s: f64,
        end_s: f64,
    },
    /// A recognizer pass came back.
    Recognized {
        /// "segment" or "preview".
        pass: &'static str,
        index: Option<usize>,
        audio_s: f64,
        ms: f64,
        text: String,
    },
    /// What the Bar was told to show.
    Bar {
        committed: String,
        preview: String,
        #[serde(rename = "final")]
        is_final: bool,
    },
    KeyUp,
    /// Whether the cleanup gate sent the text to the cleanup model, and why.
    Gate {
        reason: Option<&'static str>,
    },
    /// A cleanup: "live" (while speaking, of the dictation so far), "final" (at key up), or
    /// "reused" (at key up, the last live cleanup already covered everything).
    Cleanup {
        when: &'static str,
        input: String,
        output: String,
        ms: f64,
        error: Option<String>,
    },
    Inserted {
        ok: bool,
        error: Option<String>,
    },
    Cancelled,
}

#[derive(Debug, Serialize)]
struct Timed {
    t_ms: f64,
    #[serde(flatten)]
    entry: Entry,
}

#[derive(Debug, Serialize)]
struct Target {
    title: String,
    process_id: u32,
}

#[derive(Debug, Serialize)]
struct Record<'a> {
    /// Milliseconds since the Unix epoch at key down; also the file name.
    id: u128,
    target: Option<Target>,
    config: &'a serde_json::Value,
    raw: Option<&'a str>,
    text: Option<&'a str>,
    audio_s: f64,
    audio_file: Option<String>,
    timings_ms: Option<Timings>,
    events: Vec<Timed>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Timings {
    pub asr: f64,
    pub cleanup: f64,
    pub total: f64,
}

/// Collects one dictation's entries while it runs; `finish` writes them.
pub struct DictationLog {
    cfg: LogConfig,
    id: u128,
    start: Instant,
    events: Vec<Timed>,
}

impl DictationLog {
    pub fn start(cfg: LogConfig) -> Self {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        Self { cfg, id, start: Instant::now(), events: Vec::new() }
    }

    pub fn push(&mut self, entry: Entry) {
        if self.cfg.enabled {
            self.events.push(Timed { t_ms: self.start.elapsed().as_secs_f64() * 1e3, entry });
        }
    }

    /// Writes the record (and the audio) on a background thread, off the dictation's path.
    pub fn finish(self, done: Finished) {
        if !self.cfg.enabled {
            return;
        }
        std::thread::Builder::new()
            .name("utter-log".into())
            .spawn(move || {
                if let Err(e) = self.write(done) {
                    tracing::warn!("dictation log: {e}");
                }
            })
            .ok();
    }

    fn write(self, done: Finished) -> std::io::Result<()> {
        let dir = log_dir();
        std::fs::create_dir_all(&dir)?;
        let audio_file = if self.cfg.audio && !done.audio.is_empty() {
            let name = format!("{}.wav", self.id);
            write_wav(&dir.join(&name), &done.audio)?;
            Some(name)
        } else {
            None
        };
        let record = Record {
            id: self.id,
            target: done.target.map(|(title, process_id)| Target { title, process_id }),
            config: &done.config,
            raw: done.raw.as_deref(),
            text: done.text.as_deref(),
            audio_s: done.audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
            audio_file,
            timings_ms: done.timings,
            events: self.events,
        };
        let json = serde_json::to_vec_pretty(&record).map_err(std::io::Error::other)?;
        // Written whole, then renamed: a reader never sees half a record.
        let path = dir.join(format!("{}.json", self.id));
        let part = path.with_extension("json.part");
        std::fs::write(&part, json)?;
        std::fs::rename(part, path)
    }
}

/// What the log needs once the dictation is over.
pub struct Finished {
    pub target: Option<(String, u32)>,
    pub config: serde_json::Value,
    pub raw: Option<String>,
    pub text: Option<String>,
    pub audio: Vec<f32>,
    pub timings: Option<Timings>,
}

fn write_wav(path: &std::path::Path, audio: &[f32]) -> std::io::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: utter_audio::SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(std::io::Error::other)?;
    for &s in audio {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).map_err(std::io::Error::other)?;
    }
    w.finalize().map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_serialize_flat_with_their_time() {
        let t = Timed {
            t_ms: 12.5,
            entry: Entry::Bar { committed: "Hello".into(), preview: "wor".into(), is_final: false },
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "bar");
        assert_eq!(v["t_ms"], 12.5);
        assert_eq!(v["final"], false);
        assert_eq!(v["committed"], "Hello");
    }

    #[test]
    fn a_record_and_its_audio_are_written_whole() {
        let dir = std::env::temp_dir().join(format!("utter-log-test-{}", std::process::id()));
        // SAFETY: this is the only test in the crate that touches the environment.
        unsafe { std::env::set_var("UTTER_DATA_DIR", &dir) };
        let mut log = DictationLog::start(LogConfig::default());
        log.push(Entry::KeyUp);
        log.push(Entry::Gate { reason: Some("filler") });
        let id = log.id;
        log.write(Finished {
            target: Some(("Notepad".into(), 42)),
            config: serde_json::json!({ "dictionary": [] }),
            raw: Some("um hello".into()),
            text: Some("Hello.".into()),
            audio: vec![0.0; 16_000],
            timings: Some(Timings { asr: 30.0, cleanup: 90.0, total: 125.0 }),
        })
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(log_dir().join(format!("{id}.json"))).unwrap()).unwrap();
        assert_eq!(json["text"], "Hello.");
        assert_eq!(json["audio_s"], 1.0);
        assert_eq!(json["events"][1]["kind"], "gate");
        assert_eq!(json["events"][1]["reason"], "filler");
        assert_eq!(json["target"]["title"], "Notepad");
        let wav = hound::WavReader::open(log_dir().join(format!("{id}.wav"))).unwrap();
        assert_eq!(wav.duration(), 16_000);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
