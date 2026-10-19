//! One dictation from audio to text, the same on every platform: segments cut at pauses and
//! transcribed while the user speaks, live words, and at the end the gate and cleanup. The
//! desktop pipeline drives it from its microphone and hotkey and inserts the text; Android's
//! core (utter-mobile) drives it from the audio the app hands over.
//!
//! A [`Recognizer`] holds what lives across dictations: voice detection, the speech worker (a
//! thread that owns the recognizer and runs jobs in order) and the cleanup worker. A
//! [`Session`] is one dictation's state; each call borrows the recognizer.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use utter_asr::ParakeetTdt;
use utter_asr::boost::BoostConfig;
use utter_asr::vad::{FRAME, Segmenter, SegmenterConfig, SileroVad};
use utter_llm::cleanup;
use utter_llm::{ChatParams, LlamaServer};

use crate::PipelineError;
use crate::clean::{self, Cleaner};
use crate::gate;
use crate::log::{DictationLog, Entry};
use crate::window::{Window, joined};

/// While listening, a recognizer pass at least this often keeps the GPU's clocks up
/// (docs/eval-results.md, "Idle GPU").
const KEEPALIVE: Duration = Duration::from_millis(150);
/// Whether keep-alive passes are worth their cost: on a GPU they hold its clocks up; on the
/// processor they only took cores from the passes that matter and from the cleanup server.
fn keeps_gpu_busy() -> bool {
    utter_runtime::backend() != utter_runtime::Backend::Cpu
}
/// How often the live words are refreshed while speaking; these passes also keep the GPU busy.
const PREVIEW_EVERY: Duration = Duration::from_millis(300);
/// The least unfinished speech worth a preview pass (0.4 s).
const PREVIEW_MIN_SAMPLES: usize = 6_400;
/// The longest the paste waits for the final cleanup before inserting the raw text instead.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) enum Job {
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

pub(crate) struct Done {
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

/// The speech worker's two ends: where jobs go, where results come back.
pub(crate) type Speech = (Sender<Job>, Receiver<Done>);

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

/// Warms the recognizer, boosting `dictionary`, and starts the speech worker on it.
pub(crate) fn start_speech_worker(mut asr: ParakeetTdt, dictionary: &[String]) -> Result<Speech, PipelineError> {
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

/// The job to run, given `job` and what is queued behind it. A keep-alive or preview with work
/// queued behind it is stale: skip it. It would only delay the segment, possibly the last one,
/// which the user is waiting for, and a newer preview supersedes an older one. A keep-alive
/// never supersedes a preview, which keeps the GPU busy as well: where a pass takes longer than
/// the keep-alive interval (about 300 ms on the phone's GPU and on the emulator's CPU),
/// keep-alives queued behind each preview replaced it, and the live words stalled for seconds.
fn newest_job(mut job: Job, queued: &Receiver<Job>) -> Job {
    while matches!(job, Job::KeepAlive | Job::Preview { .. }) {
        match queued.try_recv() {
            Ok(Job::KeepAlive) if matches!(job, Job::Preview { .. }) => {}
            Ok(next) => job = next,
            Err(_) => break,
        }
    }
    job
}

/// Owns the recognizer; runs jobs in order.
fn speech_worker(mut asr: ParakeetTdt, jobs: Receiver<Job>, done: Sender<Done>) {
    let silence = vec![0.0f32; 8_000];
    while let Ok(job) = jobs.recv() {
        let job = newest_job(job, &jobs);
        let started = Instant::now();
        let (generation, kind, audio) = match job {
            Job::KeepAlive => {
                let _ = asr.transcribe(&silence);
                tracing::trace!(ms = started.elapsed().as_secs_f64() * 1e3, "keep-alive");
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
        tracing::trace!(generation, audio_s, ms, "recognized");
        if done.send(Done { generation, kind, text, audio_s, ms }).is_err() {
            break;
        }
    }
}

/// Takes over the cleanup model's server, warms it (the first request after loading is slow;
/// paid for now, not by a dictation) and starts the cleanup worker on it.
pub fn start_cleaner(
    server: LlamaServer,
    system: String,
    dictionary: Vec<String>,
    ctx: cleanup::Context,
) -> Result<Cleaner, PipelineError> {
    server.chat(&cleanup::messages_with(&system, clean::WARM_UP, &dictionary, &ctx), &ChatParams::default())?;
    Ok(Cleaner::start(server, system, dictionary, ctx))
}

fn join_parts(parts: &[String]) -> String {
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

/// What lives across dictations: voice detection, the speech worker, the cleanup worker
/// (`None` with cleanup off).
pub struct Recognizer {
    vad: SileroVad,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    pub(crate) cleaner: Option<Cleaner>,
    generation: u64,
}

impl Recognizer {
    /// Warms `asr` and starts the speech worker on it, boosting `dictionary`.
    pub fn start(
        asr: ParakeetTdt,
        vad: SileroVad,
        cleaner: Option<Cleaner>,
        dictionary: &[String],
    ) -> Result<Self, PipelineError> {
        Ok(Self::new(start_speech_worker(asr, dictionary)?, vad, cleaner))
    }

    pub(crate) fn new(speech: Speech, vad: SileroVad, cleaner: Option<Cleaner>) -> Self {
        let (jobs, done) = speech;
        Self { vad, jobs, done, cleaner, generation: 0 }
    }

    /// A new speech worker (a model change); the old one ends when its job channel closes.
    pub(crate) fn set_speech(&mut self, (jobs, done): Speech) {
        self.jobs = jobs;
        self.done = done;
    }

    /// From the next dictation on: boost `dictionary` in recognition and give it, with `ctx`,
    /// to cleanup.
    pub fn configure(&self, dictionary: &[String], ctx: cleanup::Context) {
        let _ = self.jobs.send(Job::Phrases(dictionary.to_vec()));
        if let Some(cleaner) = &self.cleaner {
            cleaner.configure(dictionary.to_vec(), ctx);
        }
    }
}

/// When a dictation's text goes through the cleanup model: always, or when the gate flags it.
#[derive(Debug, Clone)]
pub struct CleanRule {
    pub always: bool,
    /// The user's terms (the gate flags a dictation that uses one).
    pub dictionary: Vec<String>,
    /// The cleanup window, in words (crate::window).
    pub window_words: usize,
    /// The window of the cleanup waited for at key up, in words (Window::split_final).
    pub final_words: usize,
}

impl CleanRule {
    fn applies(&self, raw: &str) -> bool {
        self.always || gate::reason(raw, &self.dictionary).is_some()
    }
}

/// One dictation, from the first audio to the text.
pub struct Session {
    generation: u64,
    rule: CleanRule,
    segmenter: Segmenter,
    /// The recording from sample `audio_from` on: all of it up to [`KEEP_SAMPLES`], then only
    /// what is still to transcribe.
    audio: Vec<f32>,
    audio_from: usize,
    /// The recording's loudest sample so far.
    peak: f32,
    /// Where the segment being spoken starts, in samples from the start of the recording.
    last_cut: usize,
    /// Segments sent to the speech worker, and how many came back.
    sent: usize,
    received: usize,
    /// Each segment's transcript, in order, and all of them joined: redone when one lands, not
    /// at every poll (a long dictation's text is megabytes).
    parts: Vec<String>,
    committed: String,
    /// The segment being spoken, transcribed so far.
    preview: String,
    last_job: Instant,
    last_preview: Instant,
    /// The newest cleanup while speaking: (the raw text it cleaned, the result), and the raw
    /// text of the newest request (so the same text is never sent twice).
    latest: Option<(String, String)>,
    requested: Option<String>,
    /// The cleanup window over the dictation's end (crate::window): a request never carries
    /// more than its words and what is new, however long the dictation.
    window: Window,
    /// Requests in flight (the worker answers only the newest one queued).
    pending: Vec<Pending>,
}

/// One cleanup request while speaking: `sent` is cleaned, `whole` is the dictation so far, and
/// `start` the settled text `sent`'s cleanup follows.
struct Pending {
    sent: String,
    whole: String,
    start: String,
}

/// The longest recording kept whole (10 minutes), for retry last. A longer dictation keeps only
/// what is still to transcribe: its memory does not grow with its length.
const KEEP_SAMPLES: usize = 10 * 60 * utter_audio::SAMPLE_RATE as usize;

/// What a dictation came to.
#[derive(Debug, Clone)]
pub struct Heard {
    /// What was said, as recognized.
    pub raw: String,
    /// The text that goes in: cleaned, or `raw` when cleanup was skipped or failed.
    pub text: String,
    pub gate: Option<&'static str>,
    /// The whole recording, for retry last; empty past [`KEEP_SAMPLES`], when only what was
    /// still to transcribe was kept.
    pub audio: Vec<f32>,
    /// The recording's length in samples, and its loudest sample.
    pub samples: usize,
    pub peak: f32,
    pub segments: usize,
    /// The end of speech -> every segment transcribed.
    pub asr_ms: f64,
    /// Cleanup request time (0 when the gate skipped it).
    pub cleanup_ms: f64,
}

impl Session {
    pub fn start(rec: &mut Recognizer, rule: CleanRule) -> Self {
        rec.generation += 1;
        rec.vad.reset();
        if keeps_gpu_busy() {
            let _ = rec.jobs.send(Job::KeepAlive);
        }
        let now = Instant::now();
        let window = Window::new(rule.window_words);
        Self {
            generation: rec.generation,
            rule,
            segmenter: Segmenter::new(SegmenterConfig::default()),
            audio: Vec::new(),
            audio_from: 0,
            peak: 0.0,
            last_cut: 0,
            sent: 0,
            received: 0,
            parts: Vec::new(),
            committed: String::new(),
            preview: String::new(),
            last_job: now,
            last_preview: now,
            latest: None,
            requested: None,
            window,
            pending: Vec::new(),
        }
    }

    /// Asks for the cleanup of `whole` (of its window, when long).
    fn request(&mut self, cleaner: &clean::Cleaner, whole: String) {
        let (sent, start) = self.window.split(&whole);
        cleaner.request(self.generation, sent.clone());
        self.pending.push(Pending { sent, whole: whole.clone(), start });
        self.requested = Some(whole);
    }

    /// Takes in a cleanup that came back, as the whole dictation's text so far.
    fn take(&mut self, c: clean::Cleaned) {
        let Some(i) = self.pending.iter().rposition(|p| p.sent == c.raw) else { return };
        let p = self.pending.remove(i);
        // Requests sent before this one were passed over by the worker.
        self.pending.drain(..i);
        let ok = c.text.is_ok();
        // A failed cleanup shows the raw text: the words are never lost.
        let piece = c.text.unwrap_or(c.raw);
        let text = joined(&p.start, &piece);
        self.window.accept(&p.whole, &text, ok);
        self.latest = Some((p.whole, text));
    }

    /// The audio kept so far (all of it, unless the dictation is past [`KEEP_SAMPLES`]).
    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// Samples recorded so far.
    fn recorded(&self) -> usize {
        self.audio_from + self.audio.len()
    }

    /// Adds `new` to the recording.
    fn record(&mut self, new: &[f32]) {
        self.peak = new.iter().fold(self.peak, |m, x| m.max(x.abs()));
        self.audio.extend_from_slice(new);
    }

    /// Takes in what came back while the user speaks; returns the live words when they changed:
    /// the dictation cleaned up to the last pause (the text as it will go in, though a later
    /// correction can still change it), and what was heard after that (provisional).
    pub fn poll(&mut self, rec: &Recognizer, log: &mut DictationLog) -> Option<(String, String)> {
        let mut words_changed = false;
        let mut segment_done = false;
        while let Ok(d) = rec.done.try_recv() {
            if d.generation != self.generation {
                continue;
            }
            log.push(d.entry());
            match d.kind {
                DoneKind::Segment(i) => {
                    self.parts[i] = d.text;
                    self.received += 1;
                    segment_done = true;
                    // The preview was of this segment (or earlier audio); the next one starts
                    // fresh.
                    self.preview.clear();
                }
                DoneKind::Preview => self.preview = d.text,
            }
            words_changed = true;
        }
        if segment_done {
            self.committed = join_parts(&self.parts);
        }
        if let Some(cleaner) = &rec.cleaner {
            // A segment finished: clean the dictation so far, while the user goes on.
            if segment_done
                && !self.committed.is_empty()
                && self.requested.as_deref() != Some(self.committed.as_str())
                && self.rule.applies(&self.committed)
            {
                self.request(cleaner, self.committed.clone());
            }
            while let Some(c) = cleaner.try_recv() {
                if c.generation != self.generation {
                    continue;
                }
                log.push(c.entry("live"));
                self.take(c);
                words_changed = true;
            }
        }
        if !words_changed {
            return None;
        }
        let (committed, provisional) = if rec.cleaner.is_some() {
            let latest = self.latest.as_ref().map(|(r, t)| (r.as_str(), t.as_str()));
            clean::bar_words(latest, &self.committed, &self.preview)
        } else {
            (self.committed.clone(), self.preview.clone())
        };
        // Only the end is shown: a long dictation's text would otherwise be copied to the Bar
        // (and the log) whole at every change.
        let (committed, provisional) = (clean::shown_tail(&committed), clean::shown_tail(&provisional));
        log.push(Entry::Bar { committed: committed.clone(), preview: provisional.clone(), is_final: false });
        Some((committed, provisional))
    }

    /// Takes in audio heard since the last call: cuts segments at pauses and sends them to be
    /// transcribed, asks for live words, and keeps the GPU busy in between.
    pub fn push(&mut self, rec: &mut Recognizer, new: &[f32], log: &mut DictationLog) -> Result<(), PipelineError> {
        if new.is_empty() {
            return Ok(());
        }
        self.record(new);
        let probs = rec.vad.push(new)?;
        for cut in self.segmenter.push(&probs) {
            let cut_sample = (cut * FRAME).min(self.recorded());
            if cut_sample > self.last_cut {
                self.send_segment(rec, cut_sample, log)?;
            }
        }
        if self.last_preview.elapsed() >= PREVIEW_EVERY
            && self.segmenter.has_speech()
            && self.recorded() - self.last_cut >= PREVIEW_MIN_SAMPLES
        {
            let audio = self.audio[self.last_cut - self.audio_from..].to_vec();
            let _ = rec.jobs.send(Job::Preview { generation: self.generation, audio });
            self.last_preview = Instant::now();
            self.last_job = self.last_preview;
        } else if keeps_gpu_busy() && self.last_job.elapsed() >= KEEPALIVE {
            let _ = rec.jobs.send(Job::KeepAlive);
            self.last_job = Instant::now();
        }
        Ok(())
    }

    fn send_segment(&mut self, rec: &Recognizer, end: usize, log: &mut DictationLog) -> Result<(), PipelineError> {
        let rate = utter_audio::SAMPLE_RATE as f64;
        log.push(Entry::Segment { index: self.sent, start_s: self.last_cut as f64 / rate, end_s: end as f64 / rate });
        let audio = self.audio[self.last_cut - self.audio_from..end - self.audio_from].to_vec();
        rec.jobs
            .send(Job::Segment { generation: self.generation, index: self.sent, audio })
            .map_err(|_| PipelineError::WorkerGone)?;
        self.parts.push(String::new());
        self.sent += 1;
        self.last_cut = end;
        // Past KEEP_SAMPLES, what was transcribed is let go.
        if self.recorded() > KEEP_SAMPLES {
            self.audio.drain(..self.last_cut - self.audio_from);
            self.audio_from = self.last_cut;
        }
        self.last_job = Instant::now();
        Ok(())
    }

    /// The end of speech, at `ended` (key up), with `tail` the audio not yet pushed: transcribes
    /// the last segment, waits for every segment, then applies the gate and cleanup. The
    /// newest cleanup while speaking is the final text when nothing was heard after it.
    pub fn finish(
        mut self,
        rec: &mut Recognizer,
        tail: &[f32],
        ended: Instant,
        log: &mut DictationLog,
    ) -> Result<Heard, PipelineError> {
        self.record(tail);
        if self.recorded() > self.last_cut {
            self.send_segment(rec, self.recorded(), log)?;
        }
        while self.received < self.sent {
            let d = rec.done.recv().map_err(|_| PipelineError::WorkerGone)?;
            if d.generation != self.generation {
                continue;
            }
            log.push(d.entry());
            if let DoneKind::Segment(i) = d.kind {
                self.parts[i] = d.text;
                self.received += 1;
            }
        }
        let asr_ms = ended.elapsed().as_secs_f64() * 1e3;
        let raw = join_parts(&self.parts);

        // The gate is logged either way; it decides only when not every dictation is cleaned.
        let gate = if raw.is_empty() { None } else { gate::reason(&raw, &self.rule.dictionary) };
        log.push(Entry::Gate { reason: gate });
        let started = Instant::now();
        let text = match &rec.cleaner {
            Some(cleaner) if !raw.is_empty() && self.rule.applies(&raw) => match self.latest.take() {
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
                    // A long dictation: only the key-up window is cleaned now.
                    let (sent, start) = self.window.split_final(&raw, self.rule.final_words);
                    if !self.pending.iter().any(|p| p.whole == raw && p.sent == sent) {
                        self.request(cleaner, raw.clone());
                    }
                    match cleaner.wait_for(self.generation, &sent, CLEANUP_TIMEOUT) {
                        Some(c) => {
                            log.push(c.entry("final"));
                            // The words are never lost to a cleanup failure: the raw text goes in.
                            let piece = c.text.unwrap_or_else(|e| {
                                tracing::warn!("cleanup failed, inserting the raw text: {e}");
                                sent.clone()
                            });
                            joined(&start, &piece)
                        }
                        None => {
                            tracing::warn!("cleanup took over {CLEANUP_TIMEOUT:?}; inserting the raw text");
                            log.push(Entry::Cleanup {
                                when: "final",
                                input: sent.clone(),
                                output: sent.clone(),
                                ms: started.elapsed().as_secs_f64() * 1e3,
                                error: Some("timed out".into()),
                            });
                            joined(&start, &sent)
                        }
                    }
                }
            },
            _ => raw.clone(),
        };
        let cleanup_ms = started.elapsed().as_secs_f64() * 1e3;
        let samples = self.recorded();
        // Kept whole, or let go of: never a part posing as the whole.
        let audio = if self.audio_from == 0 { self.audio } else { Vec::new() };
        Ok(Heard { raw, text, gate, audio, samples, peak: self.peak, segments: self.sent, asr_ms, cleanup_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(jobs: Vec<Job>) -> Receiver<Job> {
        let (tx, rx) = mpsc::channel();
        for j in jobs {
            tx.send(j).unwrap();
        }
        rx
    }

    fn preview(generation: u64) -> Job {
        Job::Preview { generation, audio: vec![] }
    }

    fn bare_session() -> Session {
        let now = Instant::now();
        Session {
            generation: 1,
            rule: CleanRule { always: true, dictionary: Vec::new(), window_words: 100, final_words: 30 },
            segmenter: Segmenter::new(SegmenterConfig::default()),
            audio: Vec::new(),
            audio_from: 0,
            peak: 0.0,
            last_cut: 0,
            sent: 0,
            received: 0,
            parts: Vec::new(),
            committed: String::new(),
            preview: String::new(),
            last_job: now,
            last_preview: now,
            latest: None,
            requested: None,
            window: Window::new(100),
            pending: Vec::new(),
        }
    }

    fn cleaned(raw: &str, text: Result<&str, &str>) -> clean::Cleaned {
        clean::Cleaned { generation: 1, raw: raw.into(), text: text.map(str::to_owned).map_err(str::to_owned), ms: 0.0 }
    }

    #[test]
    fn a_cleanup_that_comes_back_is_the_whole_text_so_far() {
        let mut s = bare_session();
        s.pending.push(Pending { sent: "um hi there".into(), whole: "um hi there".into(), start: String::new() });
        s.take(cleaned("um hi there", Ok("Hi there.")));
        assert_eq!(s.latest, Some(("um hi there".to_owned(), "Hi there.".to_owned())));
        // A failed piece shows its raw words after the cleaned start.
        s.pending.push(Pending {
            sent: "uh more".into(),
            whole: "um hi there uh more".into(),
            start: "Hi there.".into(),
        });
        s.take(cleaned("uh more", Err("timed out")));
        assert_eq!(s.latest.as_ref().map(|(_, t)| t.as_str()), Some("Hi there. uh more"));
    }

    #[test]
    fn a_keep_alive_never_replaces_a_preview() {
        let rx = queue(vec![Job::KeepAlive, Job::KeepAlive]);
        assert!(matches!(newest_job(preview(1), &rx), Job::Preview { generation: 1, .. }));
    }

    #[test]
    fn a_newer_preview_replaces_an_older_one_past_keep_alives() {
        let rx = queue(vec![Job::KeepAlive, preview(2), Job::KeepAlive]);
        assert!(matches!(newest_job(preview(1), &rx), Job::Preview { generation: 2, .. }));
    }

    #[test]
    fn a_segment_replaces_whatever_is_stale_before_it() {
        let rx = queue(vec![preview(1), Job::Segment { generation: 1, index: 0, audio: vec![] }, preview(1)]);
        assert!(matches!(newest_job(Job::KeepAlive, &rx), Job::Segment { index: 0, .. }));
        // The preview after the segment waits for the next turn.
        assert!(matches!(rx.try_recv(), Ok(Job::Preview { .. })));
    }

    #[test]
    fn a_keep_alive_alone_runs() {
        let rx = queue(vec![]);
        assert!(matches!(newest_job(Job::KeepAlive, &rx), Job::KeepAlive));
    }
}
