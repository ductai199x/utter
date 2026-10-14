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

pub(crate) struct Cleaner {
    jobs: Sender<Job>,
    done: Receiver<Cleaned>,
    health: Receiver<Health>,
    prompt: Prompt,
}

impl Cleaner {
    /// Takes over the server; `system`, `dictionary` and `ctx` are the prompt's fixed parts.
    pub fn start(server: LlamaServer, system: String, dictionary: Vec<String>, ctx: cleanup::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Cleaned>();
        let (health_tx, health) = mpsc::channel::<Health>();
        let prompt: Prompt = Arc::new(Mutex::new((dictionary, ctx)));
        let worker = Worker { server, system, prompt: prompt.clone(), done: done_tx, health: health_tx };
        std::thread::Builder::new()
            .name("utter-cleanup".into())
            .spawn(move || worker.run(job_rx))
            .expect("spawn the cleanup worker");
        Self { jobs, done, health, prompt }
    }

    /// A change of the server's state since the last call, if any.
    pub fn try_health(&self) -> Option<Health> {
        self.health.try_recv().ok()
    }

    /// From the next request on, prompts carry this dictionary and context.
    pub fn configure(&self, dictionary: Vec<String>, ctx: cleanup::Context) {
        *self.prompt.lock().expect("cleanup prompt lock") = (dictionary, ctx);
    }

    pub fn request(&self, generation: u64, raw: String) {
        let _ = self.jobs.send(Job { generation, raw });
    }

    pub fn try_recv(&self) -> Option<Cleaned> {
        self.done.try_recv().ok()
    }

    /// Waits for the cleanup of exactly `raw` in `generation`, up to `timeout`. Results for
    /// anything else that arrive meanwhile are dropped: at key up, only this one matters.
    pub fn wait_for(&self, generation: u64, raw: &str, timeout: Duration) -> Option<Cleaned> {
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
        let params = ChatParams { max_tokens: cleanup::max_tokens(&job.raw), ..ChatParams::default() };
        let messages = {
            let (dictionary, ctx) = &*self.prompt.lock().expect("cleanup prompt lock");
            cleanup::messages_with(&self.system, &job.raw, &crate::gate::relevant_terms(&job.raw, dictionary), ctx)
        };
        let text = cleanup::chat(&self.server, &messages, &params)
            .map(|out| {
                if out.retried {
                    tracing::warn!("the cleanup reply ran away; the guarded second try answered");
                }
                cleanup::tidy_output(&out.completion.text)
            })
            .map_err(|e| e.to_string())
            // A reply instead of the dictation: the transcript is typed instead (logged as an error).
            .and_then(|text| {
                if cleanup::strayed(&job.raw, &text) {
                    Err(format!("output strayed from the dictation: {text}"))
                } else {
                    Ok(text)
                }
            });
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

/// What a fresh server cleans first, so the first dictation does not pay for loading.
pub(crate) const WARM_UP: &str = "Um, this is a warm-up, no, a second warm-up.";

/// What the Bar shows: the cleaned text, then the words heard since that cleanup started
/// (provisional). `latest` is the newest cleanup (its raw input and result); `committed_raw` is
/// every finished segment's raw text; `preview` the segment being spoken.
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
