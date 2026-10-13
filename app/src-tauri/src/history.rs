//! The Home window's History section and paste rules (docs/ux/history.md, docs/ux/insertion.md):
//! commands over the pipeline's History store, which the page reads a page at a time.

use serde::Serialize;
use utter_pipeline::history::{Change, Entry, History, Page, Query};
use utter_pipeline::settings::{KeepHistory, PasteOverride};
use utter_platform::focus;
use utter_platform::insert::{self, PasteRule};

#[tauri::command]
pub fn history_query(query: Query) -> Result<Page, String> {
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
pub fn history_count_older(keep: KeepHistory) -> Result<usize, String> {
    match keep.max_age() {
        None => Ok(0),
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
    /// How many dictations went to this program (History).
    pub dictations: usize,
}

/// An app the person can add a rule for.
#[derive(Serialize)]
pub struct AppRow {
    pub exe: String,
    pub name: String,
    /// Dictations into it (History); 0 for an app only open now.
    pub dictations: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteRules {
    /// Utter's defaults with the person's changes over them, then the person's own rules.
    pub rules: Vec<RuleRow>,
    /// Apps open now, for "Add an app".
    pub open_apps: Vec<AppRow>,
    /// Apps dictated into, most used first (History), for "Add an app".
    pub recent_apps: Vec<AppRow>,
    /// The app the last dictation went to: the quick path when a paste did not work.
    pub last_app: Option<AppRow>,
    /// The last dictation did not paste where it went (the keys went but nothing showed, or
    /// they could not be sent): a paste rule may fix that app.
    pub last_failed: bool,
}

fn words(rule: PasteRule) -> (String, String) {
    let o = PasteOverride::from_rule("", rule);
    (utter_pipeline::settings::word(&o.with), utter_pipeline::settings::word(&o.keep))
}

/// The rules the Settings page lists, merged from this platform's defaults and `overrides`.
#[tauri::command]
pub fn paste_rules(overrides: Vec<PasteOverride>) -> PasteRules {
    // History only adds counts and names here: the rules list without them when it cannot be read.
    let page = History::global()
        .query(&Query { limit: 1, ..Query::default() })
        .inspect_err(|e| tracing::warn!("history: {e}"))
        .unwrap_or_default();
    let used = |exe: &str| page.apps.iter().find(|a| a.exe.eq_ignore_ascii_case(exe)).map_or(0, |a| a.count);
    let mut rules: Vec<RuleRow> = Vec::new();
    for &(app, name, rule) in insert::default_rules() {
        let mine = overrides.iter().find(|o| o.app.eq_ignore_ascii_case(app));
        let (with, keep) = words(mine.map_or(rule, PasteOverride::rule));
        let dictations = used(app);
        rules.push(RuleRow { app: app.into(), name: name.into(), with, keep, default: Some(words(rule)), dictations });
    }
    let open = focus::open_apps();
    let last = page.entries.first().cloned();
    for o in &overrides {
        if rules.iter().any(|r| r.app.eq_ignore_ascii_case(&o.app)) {
            continue;
        }
        // The friendly name: from an open window, or History, else the program's file name.
        let name = open
            .iter()
            .find(|a| a.exe.eq_ignore_ascii_case(&o.app))
            .map(|a| a.name.clone())
            .or_else(|| page.apps.iter().find(|a| a.exe.eq_ignore_ascii_case(&o.app)).map(|a| a.name.clone()))
            .unwrap_or_else(|| o.app.trim_end_matches(".exe").to_owned());
        let (with, keep) = words(o.rule());
        rules.push(RuleRow { app: o.app.clone(), name, with, keep, default: None, dictations: used(&o.app) });
    }
    PasteRules {
        rules,
        open_apps: open.into_iter().map(|a| AppRow { dictations: used(&a.exe), exe: a.exe, name: a.name }).collect(),
        recent_apps: page
            .apps
            .iter()
            .map(|a| AppRow { exe: a.exe.clone(), name: a.name.clone(), dictations: a.count })
            .collect(),
        last_failed: last.as_ref().is_some_and(|e| matches!(e.not_inserted.as_deref(), Some("unconfirmed" | "failed"))),
        last_app: last.filter(|e| !e.app_exe.is_empty()).map(|e| AppRow {
            dictations: used(&e.app_exe),
            exe: e.app_exe,
            name: e.app_name,
        }),
    }
}

/// Opens the Paste rules window (docs/ux/insertion.md), or brings it forward if it is open.
/// Async: a window built inside a synchronous command deadlocks on Windows (Tauri's
/// WebviewWindowBuilder::new docs).
#[tauri::command]
pub async fn open_paste_rules(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("rules") {
        let _ = w.unminimize();
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(&app, "rules", tauri::WebviewUrl::App("rules.html".into()))
        .title("Paste rules")
        .inner_size(880.0, 620.0)
        .min_inner_size(640.0, 420.0)
        .center()
        .build()
        .map(drop)
        .map_err(|e| e.to_string())
}
