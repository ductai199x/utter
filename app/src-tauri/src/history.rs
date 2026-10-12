//! The Home window's History section and paste rules (docs/ux/history.md, docs/ux/insertion.md):
//! commands over the pipeline's History store, which the page reads a page at a time.

use serde::Serialize;
use utter_pipeline::history::{Change, Entry, History, Page, Query};
use utter_pipeline::settings::{KeepHistory, PasteOverride};
use utter_platform::focus;
use utter_platform::insert::{self, PasteRule};

#[tauri::command]
pub fn history_query(query: Query) -> Page {
    History::global().query(&query)
}

#[tauri::command]
pub fn history_favorite(id: u64, favorite: bool) -> Result<(), String> {
    History::global().set_favorite(id, favorite)
}

/// Deletes an entry and returns it, for Undo.
#[tauri::command]
pub fn history_delete(id: u64) -> Result<Option<Entry>, String> {
    History::global().delete(id)
}

#[tauri::command]
pub fn history_restore(entry: Entry) -> Result<(), String> {
    History::global().restore(entry)
}

/// The person copying: the text stays on the clipboard.
#[tauri::command]
pub fn history_copy(text: String) -> Result<(), String> {
    insert::copy(&text).map_err(|e| e.to_string())
}

/// What cleanup changed, for "What Utter heard".
#[tauri::command]
pub fn history_changes(heard: String, text: String) -> Vec<Change> {
    utter_pipeline::history::changes(&heard, &text)
}

/// How many entries a shorter keep time would delete (for its confirm).
#[tauri::command]
pub fn history_count_older(keep: KeepHistory) -> usize {
    match keep.max_age() {
        None => 0,
        Some(age) => History::global().count_older(age),
    }
}

#[tauri::command]
pub fn history_clear() -> Result<(), String> {
    History::global().clear()
}

/// A paste rule as the Settings page shows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRow {
    pub app: String,
    pub name: String,
    /// "ctrl-v", "ctrl-shift-v", "shift-insert" or "type"; "normal", "3s", "5s" or "leave".
    pub with: String,
    pub keep: String,
    /// Utter's default for this program, when it has one (for "Default" and Reset).
    pub default: Option<(String, String)>,
}

/// An app the person can add a rule for.
#[derive(Serialize)]
pub struct AppRow {
    pub exe: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteRules {
    /// Utter's defaults with the person's changes over them, then the person's own rules.
    pub rules: Vec<RuleRow>,
    /// Apps open now, for "Add an app".
    pub open_apps: Vec<AppRow>,
    /// The app the last dictation went to: the quick path when a paste did not work.
    pub last_app: Option<AppRow>,
}

fn words(rule: PasteRule) -> (String, String) {
    let o = PasteOverride::from_rule("", rule);
    (utter_pipeline::settings::word(&o.with), utter_pipeline::settings::word(&o.keep))
}

/// The rules the Settings page lists, merged from this platform's defaults and `overrides`.
#[tauri::command]
pub fn paste_rules(overrides: Vec<PasteOverride>) -> PasteRules {
    let mut rules: Vec<RuleRow> = Vec::new();
    for &(app, name, rule) in insert::default_rules() {
        let mine = overrides.iter().find(|o| o.app.eq_ignore_ascii_case(app));
        let (with, keep) = words(mine.map_or(rule, PasteOverride::rule));
        rules.push(RuleRow { app: app.into(), name: name.into(), with, keep, default: Some(words(rule)) });
    }
    let open = focus::open_apps();
    let last = History::global().query(&Query { limit: 1, ..Query::default() }).entries.into_iter().next();
    for o in &overrides {
        if rules.iter().any(|r| r.app.eq_ignore_ascii_case(&o.app)) {
            continue;
        }
        // The friendly name: from an open window, or History, else the program's file name.
        let name = open
            .iter()
            .find(|a| a.exe.eq_ignore_ascii_case(&o.app))
            .map(|a| a.name.clone())
            .or_else(|| last.as_ref().filter(|e| e.app_exe.eq_ignore_ascii_case(&o.app)).map(|e| e.app_name.clone()))
            .unwrap_or_else(|| o.app.trim_end_matches(".exe").to_owned());
        let (with, keep) = words(o.rule());
        rules.push(RuleRow { app: o.app.clone(), name, with, keep, default: None });
    }
    PasteRules {
        rules,
        open_apps: open.into_iter().map(|a| AppRow { exe: a.exe, name: a.name }).collect(),
        last_app: last.filter(|e| !e.app_exe.is_empty()).map(|e| AppRow { exe: e.app_exe, name: e.app_name }),
    }
}
