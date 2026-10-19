//! Cleanup while the user speaks (docs/plan-final-text.md, "One document, cleaned as you
//! speak"). Each time a segment is transcribed, the dictation so far is sent for cleanup; the
//! Bar shows the cleaned text as soon as it is back, with the words not yet cleaned after it,
//! marked provisional. At key up the final text is the cleanup of the whole dictation: already
//! done if nothing new was heard since the last pause, so the paste does not wait for it.
//!
//! A worker thread owns the cleanup model's server. Requests carry the exact raw text they
//! clean; a newer request makes older queued ones stale, and they are skipped.
//!
//! The worker also keeps the server alive: it looks every [`HEALTH_CHECK`] while idle and after
//! any failed request, and restarts a server that has exited (a crash, a GPU driver reset),
//! waiting longer after each failed try. While it is down every request fails at once, so the
//! raw text goes in without waiting, and the pipeline hears [`Health`] changes to say so.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use utter_llm::cleanup;
use utter_llm::{ChatParams, LlamaServer};

struct Job {
    generation: u64,
    raw: String,
}

/// A finished cleanup: of `raw`, for dictation `generation`.
pub(crate) struct Cleaned {
    pub generation: u64,
    pub raw: String,
    pub text: Result<String, String>,
    pub ms: f64,
}

impl Cleaned {
    pub fn entry(&self, when: &'static str) -> crate::log::Entry {
        let (output, error) = match &self.text {
            Ok(t) => (t.clone(), None),
            Err(e) => (self.raw.clone(), Some(e.clone())),
        };
        crate::log::Entry::Cleanup { when, input: self.raw.clone(), output, ms: self.ms, error }
    }
}

/// The user's part of every prompt: their dictionary and the context lines. Settings can
/// change it between requests.
pub(crate) type Prompt = Arc<Mutex<(Vec<String>, cleanup::Context)>>;

/// How often an idle worker checks that its server still runs.
const HEALTH_CHECK: Duration = Duration::from_secs(5);
/// Waits before each restart try: the first at once, then longer, then every minute.
const RESTART_WAITS: [Duration; 4] =
    [Duration::from_secs(1), Duration::from_secs(5), Duration::from_secs(30), Duration::from_secs(60)];

/// The server's state as the pipeline hears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Health {
    /// It exited (`why`); restarting.
    Down { why: String },
    /// Restarted and serving again.
    Up,
}

pub struct Cleaner {
    /// None once dropping: the worker sees its job channel close and ends.
    jobs: Option<Sender<Job>>,
    done: Receiver<Cleaned>,
    health: Receiver<Health>,
    prompt: Prompt,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Cleaner {
    /// Takes over the server; `system`, `dictionary` and `ctx` are the prompt's fixed parts.
    pub fn start(server: LlamaServer, system: String, dictionary: Vec<String>, ctx: cleanup::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Cleaned>();
        let (health_tx, health) = mpsc::channel::<Health>();
        let prompt: Prompt = Arc::new(Mutex::new((dictionary, ctx)));
        let worker = Worker { server, system, prompt: prompt.clone(), done: done_tx, health: health_tx };
        let worker = std::thread::Builder::new()
            .name("utter-cleanup".into())
            .spawn(move || worker.run(job_rx))
            .expect("spawn the cleanup worker");
        Self { jobs: Some(jobs), done, health, prompt, worker: Some(worker) }
    }

    /// A change of the server's state since the last call, if any.
    pub(crate) fn try_health(&self) -> Option<Health> {
        self.health.try_recv().ok()
    }

    /// From the next request on, prompts carry this dictionary and context.
    pub(crate) fn configure(&self, dictionary: Vec<String>, ctx: cleanup::Context) {
        *self.prompt.lock().expect("cleanup prompt lock") = (dictionary, ctx);
    }

    pub(crate) fn request(&self, generation: u64, raw: String) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job { generation, raw });
        }
    }

    pub(crate) fn try_recv(&self) -> Option<Cleaned> {
        self.done.try_recv().ok()
    }

    /// Waits for the cleanup of exactly `raw` in `generation`, up to `timeout`. Results for
    /// anything else that arrive meanwhile are dropped: at key up, only this one matters.
    pub(crate) fn wait_for(&self, generation: u64, raw: &str, timeout: Duration) -> Option<Cleaned> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.done.recv_timeout(left) {
                Ok(c) if c.generation == generation && c.raw == raw => return Some(c),
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

impl Drop for Cleaner {
    /// Waits for the worker to end, and with it the server it owns: a process that exits right
    /// after (Android's replay, which leaves without exit hooks) otherwise left llama-server
    /// running, orphaned. An idle worker ends at once; one mid-request, when the reply is in.
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The cleanup worker: the server, the fixed and the person's parts of the prompt, and where
/// results and health changes go.
struct Worker {
    server: LlamaServer,
    system: String,
    prompt: Prompt,
    done: Sender<Cleaned>,
    health: Sender<Health>,
}

impl Worker {
    fn run(mut self, jobs: Receiver<Job>) {
        loop {
            let mut job = match jobs.recv_timeout(HEALTH_CHECK) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => {
                    if !self.recover(&jobs) {
                        return;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            // Only the newest request matters: the dictation has grown since the older ones.
            while let Ok(next) = jobs.try_recv() {
                job = next;
            }
            let cleaned = self.clean(job);
            let failed = cleaned.text.is_err();
            if self.done.send(cleaned).is_err() || (failed && !self.recover(&jobs)) {
                return;
            }
        }
    }

    fn clean(&self, job: Job) -> Cleaned {
        let started = Instant::now();
        let text = {
            let (dictionary, ctx) = &*self.prompt.lock().expect("cleanup prompt lock");
            clean_once(&self.server, &self.system, &job.raw, dictionary, ctx)
        };
        Cleaned { generation: job.generation, raw: job.raw, text, ms: started.elapsed().as_secs_f64() * 1e3 }
    }

    /// If the server has exited, restarts it, trying again with longer waits until it runs;
    /// meanwhile every request fails at once. Returns false when the pipeline is gone.
    fn recover(&mut self, jobs: &Receiver<Job>) -> bool {
        let Some(why) = self.server.exited() else { return true };
        tracing::error!("the cleanup server exited ({why}); restarting it");
        let _ = self.health.send(Health::Down { why });
        let mut attempt = 0;
        loop {
            // The wait answers requests as they come: the raw text goes in without a timeout.
            let until = Instant::now() + RESTART_WAITS[attempt.min(RESTART_WAITS.len() - 1)];
            attempt += 1;
            loop {
                match jobs.recv_timeout(until.saturating_duration_since(Instant::now())) {
                    Ok(job) => {
                        let text = Err("the cleanup model is restarting".to_owned());
                        if self.done.send(Cleaned { generation: job.generation, raw: job.raw, text, ms: 0.0 }).is_err()
                        {
                            return false;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return false,
                }
            }
            match self.server.restart().and_then(|()| self.warm()) {
                Ok(()) => {
                    tracing::info!(attempt, "the cleanup server is back");
                    let _ = self.health.send(Health::Up);
                    return true;
                }
                Err(e) => tracing::warn!(attempt, "the cleanup server did not restart: {e}"),
            }
        }
    }

    /// The first request after loading is slow: paid for now, not by a dictation.
    fn warm(&self) -> Result<(), utter_llm::LlmError> {
        let (dictionary, ctx) = self.prompt.lock().expect("cleanup prompt lock").clone();
        let messages = cleanup::messages_with(&self.system, WARM_UP, &dictionary, &ctx);
        self.server.chat(&messages, &ChatParams::default()).map(drop)
    }
}

/// Cleans `raw` as every dictation is cleaned: the reply limit, the guarded second try, the
/// output tidied, and a reply that strayed from the dictation refused (the caller types the
/// raw words instead).
pub fn clean_once(
    server: &LlamaServer,
    system: &str,
    raw: &str,
    dictionary: &[String],
    ctx: &cleanup::Context,
) -> Result<String, String> {
    let params = ChatParams { max_tokens: cleanup::max_tokens(raw), ..ChatParams::default() };
    let messages = cleanup::messages_with(system, raw, &crate::gate::relevant_terms(raw, dictionary), ctx);
    cleanup::chat(server, &messages, &params)
        .map(|out| {
            if out.retried {
                tracing::warn!("the cleanup reply ran away; the guarded second try answered");
            }
            cleanup::tidy_output(&out.completion.text)
        })
        .map_err(|e| e.to_string())
        // A reply instead of the dictation: the transcript is typed instead (logged as an error).
        .and_then(|text| {
            if cleanup::strayed(raw, &text) {
                Err(format!("output strayed from the dictation: {text}"))
            } else {
                Ok(text)
            }
        })
}

/// What a fresh server cleans first, so the first dictation does not pay for loading.
pub(crate) const WARM_UP: &str = "Um, this is a warm-up, no, a second warm-up.";

/// What the Bar shows: the cleaned text, then the words heard since that cleanup started
/// (provisional). `latest` is the newest cleanup (its raw input and result); `committed_raw` is
/// every finished segment's raw text; `preview` the segment being spoken.
/// The live words keep at most this many characters: the Bar shows the end of the text, and a
/// dictation can run for hours.
pub const SHOWN_CHARS: usize = 600;

/// The end of `text`, at most [`SHOWN_CHARS`] characters, starting at a word, with an ellipsis
/// when something was left out.
pub fn shown_tail(text: &str) -> String {
    if text.chars().count() <= SHOWN_CHARS {
        return text.to_owned();
    }
    let start = text.char_indices().rev().nth(SHOWN_CHARS - 1).map_or(0, |(i, _)| i);
    let rest = &text[start..];
    let rest = rest.find(' ').map_or(rest, |i| &rest[i + 1..]);
    format!("\u{2026}{rest}")
}

pub(crate) fn bar_words(latest: Option<(&str, &str)>, committed_raw: &str, preview: &str) -> (String, String) {
    let (cleaned, pending) = match latest {
        // The newest cleanup covers a prefix of what is committed: the rest is still raw.
        Some((raw, text)) if committed_raw.starts_with(raw) => (text.to_owned(), committed_raw[raw.len()..].trim()),
        _ => (String::new(), committed_raw.trim()),
    };
    let provisional = [pending, preview.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    (cleaned, provisional)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_words_show_the_end_of_a_long_text() {
        assert_eq!(shown_tail("short text"), "short text");
        let long = "word ".repeat(1000);
        let shown = shown_tail(long.trim());
        assert!(shown.starts_with('\u{2026}') && shown.ends_with("word"));
        assert!(shown.chars().count() <= SHOWN_CHARS + 1);
        assert!(!shown["\u{2026}".len()..].starts_with(' '));
    }

    #[test]
    fn before_any_cleanup_everything_is_provisional() {
        assert_eq!(bar_words(None, "um so we ship", "on fri"), (String::new(), "um so we ship on fri".into()));
    }

    #[test]
    fn the_cleaned_prefix_is_followed_by_the_words_heard_since() {
        let (cleaned, provisional) =
            bar_words(Some(("um so we ship it", "We ship it.")), "um so we ship it on friday no wait", "monday");
        assert_eq!(cleaned, "We ship it.");
        assert_eq!(provisional, "on friday no wait monday");
    }

    #[test]
    fn a_cleanup_of_everything_leaves_only_the_preview_provisional() {
        let (cleaned, provisional) = bar_words(Some(("we ship it", "We ship it.")), "we ship it", "");
        assert_eq!((cleaned.as_str(), provisional.as_str()), ("We ship it.", ""));
    }
}
