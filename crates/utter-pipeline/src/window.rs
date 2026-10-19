//! Cleaning a dictation of any length: in a window over its end.
//!
//! The cleanup model is given the last [`Window::words`] words of what was said (and anything
//! newer), never the whole dictation: a request stays the same size however long the dictation
//! runs, so cleanup keeps up with speech, the text is ready as fast at key up after an hour as
//! after a minute, and the server's context (4096 tokens, prompt and reply included) is never
//! exceeded. What falls out of the window behind is settled: the cleanup that covered it is kept
//! as it is and the window's cleanup is appended to it. The window is wide enough that a
//! correction ("scratch that", "no, I mean") reaches back to what it corrects.

/// The window, in words: about 1,000 tokens, so the model sees a long stretch of the dictation
/// at once, and its prompt, the window and its reply still fit the server's 4096 tokens.
pub const DEFAULT_WORDS: usize = 500;
/// The key-up window, in words: as accurate as 45 or 60 or the whole window, and the shortest
/// wait (Quality 0.2 s, Light 0.15 s on CUDA; Extra light 0.4 s on the processor).
pub const DEFAULT_FINAL_WORDS: usize = 30;

/// A cleanup result: the dictation it covered, its text, and the dictation's length in words.
#[derive(Debug, Clone, PartialEq)]
struct Done {
    raw: String,
    text: String,
    words: usize,
}

/// The window over a dictation's end, and the settled start before it.
#[derive(Debug)]
pub struct Window {
    words: usize,
    settled: Option<Done>,
    /// Cleanups that came back, oldest first, not yet settled.
    done: Vec<Done>,
}

impl Window {
    /// A window of `words` words.
    pub fn new(words: usize) -> Self {
        Self { words, settled: None, done: Vec::new() }
    }

    /// What to send to clean `whole` (the dictation so far): its part after the settled start,
    /// and the settled start's text, which the cleanup of that part follows ("" when nothing is
    /// settled and `whole` is sent as it is).
    pub fn split(&self, whole: &str) -> (String, String) {
        match &self.settled {
            Some(s) if whole.len() > s.raw.len() && whole.starts_with(s.raw.as_str()) => {
                (whole[s.raw.len()..].trim().to_owned(), s.text.clone())
            }
            _ => (whole.to_owned(), String::new()),
        }
    }

    /// The cleanup of `whole` came back as `text` (the whole dictation's text so far); `ok`:
    /// the model cleaned it (a failure's raw words are shown, never settled). Settles the newest
    /// cleanup that is at least a window behind.
    pub fn accept(&mut self, whole: &str, text: &str, ok: bool) {
        if !ok {
            return;
        }
        let words = whole.split_whitespace().count();
        self.done.push(Done { raw: whole.to_owned(), text: text.to_owned(), words });
        let behind = words.saturating_sub(self.words);
        if let Some(i) = self.done.iter().rposition(|d| d.words <= behind && boundary(whole, d)) {
            self.settled = Some(self.done.remove(i));
            self.done.drain(..i);
        }
    }

    /// [`Window::split`] for the cleanup the person waits for, at key up: from the newest
    /// cleanup at least `words` behind, so the wait covers a short window (its reply is as long
    /// as what it cleans) while what was said before it keeps the wide window's cleanup.
    pub fn split_final(&self, whole: &str, words: usize) -> (String, String) {
        let behind = whole.split_whitespace().count().saturating_sub(words);
        let from = self.done.iter().rev().chain(self.settled.iter()).find(|d| d.words <= behind && boundary(whole, d));
        match from {
            Some(d) => (whole[d.raw.len()..].trim().to_owned(), d.text.clone()),
            None => self.split(whole),
        }
    }

    /// The settled start's length in words (0: nothing settled yet).
    pub fn settled_words(&self) -> usize {
        self.settled.as_ref().map_or(0, |s| s.words)
    }
}

/// Whether the cleanup `d` of a start of `whole` can be kept as it is, with the rest cleaned
/// after it: never when what follows takes back or continues what came before ("Scratch that.",
/// "Second, ..."), which the cleanup after the boundary could not reach.
fn boundary(whole: &str, d: &Done) -> bool {
    whole.len() > d.raw.len() && whole.starts_with(d.raw.as_str()) && !crate::gate::refers_back(&whole[d.raw.len()..])
}

/// `a` then `b`, as one text.
pub fn joined(a: &str, b: &str) -> String {
    match (a.trim().is_empty(), b.trim().is_empty()) {
        (true, _) => b.trim().to_owned(),
        (_, true) => a.trim().to_owned(),
        _ => format!("{} {}", a.trim(), b.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize, w: &str) -> String {
        vec![w; n].join(" ")
    }

    #[test]
    fn a_short_dictation_is_cleaned_whole() {
        let mut w = Window::new(50);
        w.accept("um hi there", "Hi there.", true);
        assert_eq!(w.split("um hi there and more"), ("um hi there and more".to_owned(), String::new()));
    }

    #[test]
    fn what_falls_a_window_behind_settles_and_only_the_window_is_sent() {
        let mut w = Window::new(50);
        let a = words(30, "a");
        w.accept(&a, "A.", true);
        let b = format!("{a} {}", words(30, "b"));
        w.accept(&b, "A. B.", true); // 60 words: `a` is not 50 behind yet
        assert_eq!(w.settled_words(), 0);
        let c = format!("{b} {}", words(30, "c"));
        w.accept(&c, "A. B. C.", true); // 90 words: `a` (30) is 60 behind
        assert_eq!(w.settled_words(), 30);
        let d = format!("{c} {}", words(5, "d"));
        let (sent, start) = w.split(&d);
        assert_eq!(start, "A.");
        assert_eq!(sent.split_whitespace().count(), 65, "the window (b, c) and what is new (d)");
    }

    #[test]
    fn the_cleanup_at_key_up_covers_only_its_own_window() {
        let mut w = Window::new(200);
        let mut whole = String::new();
        let mut text = String::new();
        for i in 0..10 {
            whole = joined(&whole, &words(20, &format!("s{i}")));
            text = joined(&text, &format!("S{i}."));
            w.accept(&whole, &text, true);
        }
        let end = joined(&whole, &words(10, "end"));
        let (sent, start) = w.split_final(&end, 30);
        // From the newest cleanup at least 30 words back: 180 words in (S0..S8), then s9 and end.
        assert_eq!(sent.split_whitespace().count(), 30);
        assert!(start.ends_with("S8."));
        // A key-up window wider than the dictation cleans it whole.
        assert_eq!(w.split_final("short one", 30).0, "short one");
    }

    #[test]
    fn no_window_starts_just_before_a_correction() {
        let mut w = Window::new(200);
        let a = words(40, "a");
        w.accept(&a, "A.", true);
        let b = format!("{a} Scratch that. {}", words(40, "b"));
        w.accept(&b, "B.", true);
        // From `a`, the window would open on "Scratch that": the key-up cleanup goes back further.
        let (sent, start) = w.split_final(&format!("{b} end"), 30);
        assert!(sent.starts_with("a a") && start.is_empty(), "{sent}");
    }

    #[test]
    fn a_failed_cleanup_is_never_settled() {
        let mut w = Window::new(10);
        let a = words(20, "a");
        w.accept(&a, &a, false);
        w.accept(&format!("{a} {}", words(20, "b")), "x", false);
        assert_eq!(w.settled_words(), 0);
    }

    #[test]
    fn the_request_stays_the_window_size_however_long_the_dictation() {
        let mut w = Window::new(100);
        let mut whole = String::new();
        let mut text = String::new();
        let mut biggest = 0;
        for i in 0..500 {
            whole = joined(&whole, &words(12, &format!("w{i}")));
            let (sent, start) = w.split(&whole);
            biggest = biggest.max(sent.split_whitespace().count());
            text = joined(&start, &format!("cleaned{i}"));
            w.accept(&whole, &text, true);
        }
        assert!(biggest <= 100 + 2 * 12, "{biggest} words sent at most");
        assert!(text.ends_with("cleaned499"));
    }

    #[test]
    fn joining_skips_empty_sides() {
        assert_eq!(joined("", "B."), "B.");
        assert_eq!(joined("A.", " "), "A.");
        assert_eq!(joined("A. ", " B."), "A. B.");
    }
}
