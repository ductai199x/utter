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
#[derive(Debug, Clone, Serialize)]
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

    /// Runs `f` on the entries, loading them first if needed.
    fn with<R>(&self, f: impl FnOnce(&mut Vec<Entry>) -> R) -> R {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entries = guard.get_or_insert_with(|| load(&self.path));
        f(entries)
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
        })
    }

    /// The newest dictation with text, inserted or not: what "paste last" pastes.
    pub fn last_text(&self) -> Option<String> {
        self.with(|entries| entries.iter().rev().find(|e| !e.text.trim().is_empty()).map(|e| e.text.clone()))
    }

    pub fn query(&self, q: &Query) -> Page {
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
    pub fn count_older(&self, max_age: Duration) -> usize {
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
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Lower case without accents, so "cafe" finds "Café".
fn fold(s: &str) -> String {
    s.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// Reads the file; a line that does not parse (a crash mid-append) is skipped, not fatal.
fn load(path: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut entries: Vec<Entry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| match serde_json::from_str(l) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!("history: skipping a line that does not parse: {e}");
                None
            }
        })
        .collect();
    entries.sort_by_key(|e| e.id);
    entries
}

fn append(path: &Path, entry: &Entry) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    let mut f = OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(f, "{line}").map_err(|e| format!("{}: {e}", path.display()))
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
        let page = again.query(&Query::default());
        assert_eq!(page.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["second", "first"]);
        assert!(page.entries[0].id > page.entries[1].id);
        assert_eq!(again.last_text().as_deref(), Some("second"));
    }

    #[test]
    fn search_ignores_case_and_accents_and_reads_what_was_heard() {
        let h = temp("search");
        h.add(entry("Meet at the Café", "slack.exe", true)).unwrap();
        h.add(Entry { heard: "jira ticket".into(), ..entry("Jira ticket", "chrome.exe", true) }).unwrap();
        let find = |s: &str| h.query(&Query { search: s.into(), ..Query::default() }).matched;
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
        let page = h.query(&Query { inserted: Some(false), app: Some("SLACK.exe".into()), ..Query::default() });
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
        let page = again.query(&Query::default());
        assert_eq!(page.total, 1);
        assert!(page.entries[0].favorite);
        again.restore(removed).unwrap();
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).total, 2);
    }

    #[test]
    fn pruning_and_clearing_remove_entries() {
        let h = temp("prune");
        h.add(entry("old", "slack.exe", true)).unwrap();
        // Make it old by hand, as if written a day ago.
        h.with(|e| e[0].at -= 25 * 3600 * 1000);
        h.add(entry("new", "slack.exe", true)).unwrap();
        assert_eq!(h.count_older(Duration::from_secs(24 * 3600)), 1);
        assert_eq!(h.prune(Some(Duration::from_secs(24 * 3600))).unwrap(), 1);
        assert_eq!(h.prune(None).unwrap(), 0);
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).total, 1);
        h.clear().unwrap();
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).total, 0);
    }

    #[test]
    fn a_torn_last_line_is_skipped() {
        let h = temp("torn");
        h.add(entry("whole", "slack.exe", true)).unwrap();
        let mut f = OpenOptions::new().append(true).open(&h.path).unwrap();
        write!(f, "{{\"id\": 9, \"at\"").unwrap();
        assert_eq!(History::open(h.path.clone()).query(&Query::default()).total, 1);
    }
}
