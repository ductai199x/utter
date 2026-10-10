//! Cleanup while the user speaks (docs/plan-final-text.md, "One document, cleaned as you
//! speak"). Each time a segment is transcribed, the dictation so far is sent for cleanup; the
//! Bar shows the cleaned text as soon as it is back, with the words not yet cleaned after it,
//! marked provisional. At key up the final text is the cleanup of the whole dictation: already
//! done if nothing new was heard since the last pause, so the paste does not wait for it.
//!
//! A worker thread owns the cleanup model's client. Requests carry the exact raw text they
//! clean; a newer request makes older queued ones stale, and they are skipped.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
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

pub(crate) struct Cleaner {
    jobs: Sender<Job>,
    done: Receiver<Cleaned>,
}

impl Cleaner {
    /// Takes over the server; `system` and `dictionary` are the prompt's fixed parts.
    pub fn start(server: LlamaServer, system: String, dictionary: Vec<String>) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Cleaned>();
        std::thread::Builder::new()
            .name("utter-cleanup".into())
            .spawn(move || worker(server, system, dictionary, job_rx, done_tx))
            .expect("spawn the cleanup worker");
        Self { jobs, done }
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

fn worker(server: LlamaServer, system: String, dictionary: Vec<String>, jobs: Receiver<Job>, done: Sender<Cleaned>) {
    while let Ok(mut job) = jobs.recv() {
        // Only the newest request matters: the dictation has grown since the older ones.
        while let Ok(next) = jobs.try_recv() {
            job = next;
        }
        let started = Instant::now();
        let params = ChatParams { max_tokens: (job.raw.len() as u32 / 2).max(64), ..ChatParams::default() };
        let text = server
            .chat(&cleanup::messages_with(&system, &job.raw, &dictionary, &cleanup::Context::default()), &params)
            .map(|out| cleanup::tidy_output(&out.text))
            .map_err(|e| e.to_string());
        let cleaned =
            Cleaned { generation: job.generation, raw: job.raw, text, ms: started.elapsed().as_secs_f64() * 1e3 };
        if done.send(cleaned).is_err() {
            break;
        }
    }
}

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
