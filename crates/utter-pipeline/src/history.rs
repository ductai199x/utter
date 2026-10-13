//! History: every dictation, kept on this computer (docs/ux/history.md). One JSON object per
//! line in `<data dir>/history.jsonl`, appended as each dictation finishes; edits (favorite,
//! delete, retention) rewrite the file to a temporary one and rename it over, so a crash never
//! leaves it half written. No audio: that is the developer's log (log.rs).
//!
//! Filtering and search run here, a page at a time, so a long history never crosses to the
//! page whole. Time filters come in as instants: the page knows the local calendar.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// One dictation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Unique, and in time order.
    pub id: u64,
    /// When it finished, in milliseconds since 1970 (UTC).
    pub at: u64,
    /// The app it was for: program file name and what people call it ("" when unknown).
    pub app_exe: String,
    pub app_name: String,
    /// What was typed (or would have been).
    pub text: String,
    /// What the speech model wrote, before cleanup.
    pub heard: String,
    pub inserted: bool,
    /// Why not, when not: "focus-moved", "no-window", "refused" or "failed".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_inserted: Option<String>,
    pub words: usize,
    /// Seconds of speech, and key up to text ready in milliseconds.
    pub speech_s: f64,
    pub ready_ms: f64,
    #[serde(default)]
    pub favorite: bool,
}

/// What the History page asks for.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Query {
    pub search: String,
    pub favorites: bool,
    /// Inserted (true) or not (false); `None`: both.
    pub inserted: Option<bool>,
    /// A program file name.
    pub app: Option<String>,
    /// Only entries at or after this instant (milliseconds since 1970), and before `until`.
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub offset: usize,
    pub limit: usize,
}

/// An app in the filter column, with how many dictations went to it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCount {
    pub exe: String,
    pub name: String,
    pub count: usize,
}

/// One page of results, newest first, with what the filter column shows.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub entries: Vec<Entry>,
    /// Entries matching the query, and in all.
    pub matched: usize,
    pub total: usize,
    pub not_inserted: usize,
    pub favorites: usize,
    /// Most used first.
    pub apps: Vec<AppCount>,
}

pub struct History {
    path: PathBuf,
    /// Oldest first, loaded on first use.
    entries: Mutex<Option<Vec<Entry>>>,
}

impl History {
    /// The history in Utter's data folder: one per process, shared by the pipeline, which
    /// adds to it, and the Home window, which reads and edits it.
    pub fn global() -> &'static History {
        static HISTORY: OnceLock<History> = OnceLock::new();
        HISTORY.get_or_init(|| History::open(utter_core::data_dir().join("history.jsonl")))
    }

    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), entries: Mutex::new(None) }
    }

    /// Runs `f` on the entries, loading them first if needed. A file that could not be read
    /// is an error, and is read again next time: nothing is ever written over it.
    fn with<R>(&self, f: impl FnOnce(&mut Vec<Entry>) -> R) -> Result<R, String> {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(load(&self.path)?);
        }
        Ok(f(guard.as_mut().expect("loaded just above")))
    }

    /// Adds a finished dictation, giving it its id and time.
    pub fn add(&self, mut entry: Entry) -> Result<Entry, String> {
        self.with(|entries| {
            let now = now_ms();
            entry.at = now;
            // Ids are the time in microseconds, nudged past the last one so they stay unique.
            entry.id = (now * 1000).max(entries.last().map_or(0, |e| e.id + 1));
            append(&self.path, &entry)?;
            entries.push(entry.clone());
            Ok(entry)
        })?
    }

    /// The newest dictation with text, inserted or not: what "paste last" pastes.
    pub fn last_text(&self) -> Option<String> {
        self.with(|entries| entries.iter().rev().find(|e| !e.text.trim().is_empty()).map(|e| e.text.clone()))
            .inspect_err(|e| tracing::warn!("history: {e}"))
            .ok()
            .flatten()
    }

    pub fn query(&self, q: &Query) -> Result<Page, String> {
        self.with(|entries| {
            let needle = fold(&q.search);
            let in_time = |e: &Entry| q.since.is_none_or(|s| e.at >= s) && q.until.is_none_or(|u| e.at < u);
            let mut apps: Vec<AppCount> = Vec::new();
            for e in entries.iter().filter(|e| !e.app_exe.is_empty()) {
                match apps.iter_mut().find(|a| a.exe.eq_ignore_ascii_case(&e.app_exe)) {
                    Some(a) => a.count += 1,
                    None => apps.push(AppCount { exe: e.app_exe.clone(), name: e.app_name.clone(), count: 1 }),
                }
            }
            apps.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
            let matching: Vec<&Entry> = entries
                .iter()
                .rev()
                .filter(|e| {
                    (!q.favorites || e.favorite)
                        && q.inserted.is_none_or(|i| e.inserted == i)
                        && q.app.as_deref().is_none_or(|a| e.app_exe.eq_ignore_ascii_case(a))
                        && in_time(e)
                        && (needle.is_empty() || fold(&e.text).contains(&needle) || fold(&e.heard).contains(&needle))
                })
                .collect();
            let limit = if q.limit == 0 { 50 } else { q.limit };
            Page {
                entries: matching.iter().skip(q.offset).take(limit).map(|&e| e.clone()).collect(),
                matched: matching.len(),
                total: entries.len(),
                not_inserted: entries.iter().filter(|e| !e.inserted).count(),
                favorites: entries.iter().filter(|e| e.favorite).count(),
                apps,
            }
        })
    }

    pub fn set_favorite(&self, id: u64, favorite: bool) -> Result<(), String> {
        self.edit(|entries| {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.favorite = favorite;
            }
        })
    }

    /// Removes one entry and returns it, so it can be put back ([`History::restore`]).
    pub fn delete(&self, id: u64) -> Result<Option<Entry>, String> {
        let mut removed = None;
        self.edit(|entries| {
            if let Some(i) = entries.iter().position(|e| e.id == id) {
                removed = Some(entries.remove(i));
            }
        })?;
        Ok(removed)
    }

    /// Puts a deleted entry back where it was (Undo).
    pub fn restore(&self, entry: Entry) -> Result<(), String> {
        self.edit(|entries| {
            if entries.iter().all(|e| e.id != entry.id) {
                let at = entries.partition_point(|e| e.id < entry.id);
                entries.insert(at, entry);
            }
        })
    }

    /// How many entries are older than `max_age` (for the confirm before shortening it).
    pub fn count_older(&self, max_age: Duration) -> Result<usize, String> {
        let cutoff = now_ms().saturating_sub(max_age.as_millis() as u64);
        self.with(|entries| entries.iter().filter(|e| e.at < cutoff).count())
    }

    /// Deletes entries older than `max_age` (`None`: keep all). Returns how many went.
    pub fn prune(&self, max_age: Option<Duration>) -> Result<usize, String> {
        let Some(max_age) = max_age else { return Ok(0) };
        let cutoff = now_ms().saturating_sub(max_age.as_millis() as u64);
        let mut gone = 0;
        self.edit(|entries| {
            let before = entries.len();
            entries.retain(|e| e.at >= cutoff);
            gone = before - entries.len();
        })?;
        Ok(gone)
    }

    pub fn clear(&self) -> Result<(), String> {
        self.edit(Vec::clear)
    }

    /// Changes the entries and rewrites the file (only if something changed).
    fn edit(&self, f: impl FnOnce(&mut Vec<Entry>)) -> Result<(), String> {
        self.with(|entries| {
            let before = entries.clone();
            f(entries);
            if *entries == before {
                return Ok(());
            }
            rewrite(&self.path, entries).inspect_err(|_| *entries = before)
        })?
    }
}

/// One run of words in "what Utter heard" (docs/ux/history.md): kept, removed by cleanup, or
/// added or changed by it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// "same", "removed" or "added".
    pub kind: &'static str,
    pub text: String,
}

/// What cleanup did to what was heard, word by word: the typed text's words where they match
/// (ignoring case and punctuation, which cleanup always changes), what was heard and dropped,
/// and what was written instead. Longer texts than this compare as one replacement.
pub fn changes(heard: &str, text: &str) -> Vec<Change> {
    const MOST_WORDS: usize = 1500;
    let a: Vec<&str> = heard.split_whitespace().collect();
    let b: Vec<&str> = text.split_whitespace().collect();
    let key = |w: &str| fold(w).chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let (ka, kb): (Vec<String>, Vec<String>) = (a.iter().map(|w| key(w)).collect(), b.iter().map(|w| key(w)).collect());
    let mut out: Vec<Change> = Vec::new();
    let mut push = |kind: &'static str, word: &str| match out.last_mut() {
        Some(last) if last.kind == kind => {
            last.text.push(' ');
            last.text.push_str(word);
        }
        _ => out.push(Change { kind, text: word.to_owned() }),
    };
    if a.len() > MOST_WORDS || b.len() > MOST_WORDS {
        a.iter().for_each(|w| push("removed", w));
        b.iter().for_each(|w| push("added", w));
        return out;
    }
    // Longest common subsequence, suffix table: lcs[i][j] for a[i..], b[j..].
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if !ka[i].is_empty() && ka[i] == kb[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && !ka[i].is_empty() && ka[i] == kb[j] {
            push("same", b[j]);
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            // What was dropped reads before what replaced it.
            push("removed", a[i]);
            i += 1;
        } else {
            // A word only punctuation ("—") that cleanup added is part of the writing, not a change.
            push(if kb[j].is_empty() { "same" } else { "added" }, b[j]);
            j += 1;
        }
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Lower case without accents, so "cafe" finds "Café".
fn fold(s: &str) -> String {
    s.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// Reads the file: none yet is an empty history, any other failure an error. Bytes that are not
/// UTF-8 (a torn character) spoil only their line. A line that does not parse (a crash
/// mid-append, a newer version's entry) is moved to `history.unreadable.jsonl` beside it, never
/// lost, and the file is written again without it, so the next entry does not land on a torn
/// line.
fn load(path: &Path) -> Result<Vec<Entry>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut unreadable = String::new();
    let mut entries: Vec<Entry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| match serde_json::from_str(l) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!("history: a line that does not parse, kept aside: {e}");
                unreadable.push_str(l);
                unreadable.push('\n');
                None
            }
        })
        .collect();
    entries.sort_by_key(|e| e.id);
    if !unreadable.is_empty() {
        let aside = path.with_extension("unreadable.jsonl");
        let saved = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&aside)
            .and_then(|mut f| f.write_all(unreadable.as_bytes()));
        match saved {
            Ok(()) => {
                if let Err(e) = rewrite(path, &entries) {
                    tracing::warn!("history: {e}");
                }
            }
            Err(e) => tracing::warn!("history: could not keep the unreadable lines in {}: {e}", aside.display()),
        }
    }
    Ok(entries)
}

fn append(path: &Path, entry: &Entry) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // The line and its end in one write: a crash cannot leave a line without its newline
    // for the next entry to land on.
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())? + "\n";
    let mut f =
        OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(line.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

fn rewrite(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        text.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> History {
        let dir = std::env::temp_dir().join(format!("utter-history-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        History::open(dir.join("history.jsonl"))
    }

    fn entry(text: &str, app: &str, inserted: bool) -> Entry {
        Entry {
            id: 0,
            at: 0,
            app_exe: app.into(),
            app_name: app.trim_end_matches(".exe").into(),
            text: text.into(),
            heard: text.to_lowercase(),
            inserted,
            not_inserted: (!inserted).then(|| "focus-moved".into()),
            words: text.split_whitespace().count(),
            speech_s: 1.0,
            ready_ms: 150.0,
            favorite: false,
        }
    }

    #[test]
    fn entries_survive_a_reopen_newest_first() {
        let h = temp("reopen");
        h.add(entry("first", "slack.exe", true)).unwrap();
        h.add(entry("second", "chrome.exe", true)).unwrap();
        let again = History::open(h.path.clone());
        let page = again.query(&Query::default()).unwrap();
        assert_eq!(page.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["second", "first"]);
        assert!(page.entries[0].id > page.entries[1].id);
        assert_eq!(again.last_text().as_deref(), Some("second"));
    }

    #[test]
    fn search_ignores_case_and_accents_and_reads_what_was_heard() {
        let h = temp("search");
        h.add(entry("Meet at the Café", "slack.exe", true)).unwrap();
        h.add(Entry { heard: "jira ticket".into(), ..entry("Jira ticket", "chrome.exe", true) }).unwrap();
        let find = |s: &str| h.query(&Query { search: s.into(), ..Query::default() }).unwrap().matched;
        assert_eq!(find("cafe"), 1);
        assert_eq!(find("JIRA"), 1);
        assert_eq!(find("nothing"), 0);
    }

    #[test]
    fn filters_combine_and_apps_are_counted() {
        let h = temp("filters");
        h.add(entry("a", "slack.exe", true)).unwrap();
        h.add(entry("b", "slack.exe", false)).unwrap();
        h.add(entry("c", "chrome.exe", false)).unwrap();
        let page =
            h.query(&Query { inserted: Some(false), app: Some("SLACK.exe".into()), ..Query::default() }).unwrap();
        assert_eq!(page.matched, 1);
        assert_eq!(page.not_inserted, 2);
        assert_eq!(page.apps[0].exe, "slack.exe");
        assert_eq!(page.apps[0].count, 2);
    }

    #[test]
    fn favorite_delete_and_undo_are_saved() {
        let h = temp("edit");
        let a = h.add(entry("keep", "slack.exe", true)).unwrap();
        let b = h.add(entry("drop", "slack.exe", true)).unwrap();
        h.set_favorite(a.id, true).unwrap();
        let removed = h.delete(b.id).unwrap().unwrap();
        let again = History::open(h.path.clone());
        let page = again.query(&Query::default()).unwrap();
        assert_eq!(page.total, 1);
        assert!(page.entries[0].favorite);
        again.restore(removed).unwrap();
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).unwrap().total, 2);
    }

    #[test]
    fn pruning_and_clearing_remove_entries() {
        let h = temp("prune");
        h.add(entry("old", "slack.exe", true)).unwrap();
        // Make it old by hand, as if written a day ago.
        h.with(|e| e[0].at -= 25 * 3600 * 1000).unwrap();
        h.add(entry("new", "slack.exe", true)).unwrap();
        assert_eq!(h.count_older(Duration::from_secs(24 * 3600)).unwrap(), 1);
        assert_eq!(h.prune(Some(Duration::from_secs(24 * 3600))).unwrap(), 1);
        assert_eq!(h.prune(None).unwrap(), 0);
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).unwrap().total, 1);
        h.clear().unwrap();
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).unwrap().total, 0);
    }

    #[test]
    fn changes_mark_fillers_corrections_and_rewritten_numbers() {
        let c = changes("um so the meeting is at three no actually four", "So the meeting is at 4.");
        let shown: Vec<(&str, &str)> = c.iter().map(|c| (c.kind, c.text.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("removed", "um"),
                ("same", "So the meeting is at"),
                ("removed", "three no actually four"),
                ("added", "4."),
            ]
        );
        // Only case and punctuation changed: nothing is marked.
        assert!(changes("hello there", "Hello, there!").iter().all(|c| c.kind == "same"));
    }

    #[test]
    fn a_torn_last_line_is_kept_aside_and_the_next_entry_survives() {
        let h = temp("torn");
        h.add(entry("whole", "slack.exe", true)).unwrap();
        let mut f = OpenOptions::new().append(true).open(&h.path).unwrap();
        write!(f, "{{\"id\": 9, \"at\"").unwrap();
        drop(f);
        let reopened = History::open(h.path.clone());
        assert_eq!(reopened.query(&Query::default()).unwrap().total, 1);
        // The next entry is appended after the torn line was dealt with, and reads back.
        reopened.add(entry("after", "slack.exe", true)).unwrap();
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).unwrap().total, 2);
        let aside = std::fs::read_to_string(h.path.with_extension("unreadable.jsonl")).unwrap();
        assert_eq!(aside, "{\"id\": 9, \"at\"\n");
    }

    #[test]
    fn a_history_that_cannot_be_read_is_an_error_and_never_written_over() {
        let h = temp("unreadable");
        // A folder where the file should be: reading it fails, and not because it is missing.
        std::fs::create_dir_all(&h.path).unwrap();
        assert!(h.query(&Query::default()).is_err());
        assert!(h.add(entry("lost?", "slack.exe", true)).is_err());
        assert!(h.clear().is_err());
        assert!(h.path.is_dir(), "nothing was written in its place");
        // Once it can be read, it is: the failure was not cached as an empty history.
        std::fs::remove_dir(&h.path).unwrap();
        h.add(entry("now", "slack.exe", true)).unwrap();
        assert_eq!(h.query(&Query::default()).unwrap().total, 1);
    }

    #[test]
    fn bytes_that_are_not_utf8_spoil_only_their_line() {
        let h = temp("bytes");
        h.add(entry("first", "slack.exe", true)).unwrap();
        let mut f = OpenOptions::new().append(true).open(&h.path).unwrap();
        // A line cut inside a three-byte character.
        f.write_all(b"{\"id\": 7, \"text\": \"caf\xE2\x82\n").unwrap();
        drop(f);
        let reopened = History::open(h.path.clone());
        reopened.add(entry("second", "slack.exe", true)).unwrap();
        let page = History::open(h.path.clone()).query(&Query::default()).unwrap();
        assert_eq!(page.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["second", "first"]);
    }
}
