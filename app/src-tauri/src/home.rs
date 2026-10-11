//! The Home window's side of the core (docs/ux/home.md): commands to read and save settings,
//! and a watcher that applies `settings.toml` whenever it changes, whether the window saved it
//! or a person edited it by hand. The page renders what it is told and sends intent.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use utter_pipeline::Settings;
use utter_pipeline::settings::clean_terms;

/// How often the watcher looks at the file's modification time.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Where the watcher sends settings once the pipeline runs (it may start later, or not at all).
#[derive(Clone, Default)]
pub struct PipelineLink(pub Arc<Mutex<Option<Sender<Settings>>>>);

/// What the page needs to draw: the settings, or why the file cannot be read.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct State {
    settings: Option<Settings>,
    error: Option<String>,
    microphones: Vec<String>,
    quality_installed: bool,
    version: &'static str,
    data_dir: String,
    settings_path: String,
}

fn state() -> State {
    let path = Settings::path();
    let (settings, error) = match Settings::load_or_create(&path) {
        Ok(s) => (Some(s), None),
        Err(e) => (None, Some(e)),
    };
    let quality = utter_llm::cleanup::Tier::Quality.component();
    State {
        settings,
        error,
        microphones: utter_audio::input_devices(),
        quality_installed: utter_runtime::component(quality).map(|c| c.is_installed()).unwrap_or(false),
        version: env!("CARGO_PKG_VERSION"),
        data_dir: utter_core::data_dir().display().to_string(),
        settings_path: path.display().to_string(),
    }
}

#[tauri::command]
pub fn home_state() -> State {
    state()
}

/// Saves the settings the page sends; the watcher then applies them. Returns them as stored
/// (the dictionary cleaned), or why they could not be saved.
#[tauri::command]
pub fn save_settings(mut settings: Settings) -> Result<Settings, String> {
    settings.dictionary = clean_terms(&settings.dictionary);
    settings.save(&Settings::path())?;
    Ok(settings)
}

#[tauri::command]
pub fn open_path(which: String) -> Result<(), String> {
    let path: PathBuf = match which.as_str() {
        "settings" => Settings::path(),
        "data" => utter_core::data_dir(),
        _ => return Err(format!("unknown path {which:?}")),
    };
    utter_platform::shell::open(&path).map_err(|e| e.to_string())
}

/// Keeps the sign-in registration in step with the setting. Only an installed copy registers
/// itself: a development build (anything under a cargo `target` folder) never does.
pub fn apply_autostart(settings: &Settings) {
    let Ok(exe) = std::env::current_exe() else { return };
    if exe.components().any(|c| c.as_os_str() == "target") {
        return;
    }
    if let Err(e) = utter_platform::autostart::set(settings.start_at_login, &exe) {
        tracing::warn!("start at sign-in: {e}");
    }
}

/// Shows the Home window, focused.
pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("home") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Applies `settings.toml` on every change: to the pipeline, and to the page.
pub fn watch(app: AppHandle, link: PipelineLink) {
    std::thread::Builder::new()
        .name("utter-settings-watch".into())
        .spawn(move || {
            let path = Settings::path();
            let mut seen: Option<SystemTime> = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            loop {
                std::thread::sleep(WATCH_EVERY);
                let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if modified == seen {
                    continue;
                }
                seen = modified;
                let s = state();
                if let Some(settings) = &s.settings {
                    apply_autostart(settings);
                    if let Some(tx) = link.0.lock().expect("pipeline link").as_ref() {
                        let _ = tx.send(settings.clone());
                    }
                }
                if let Some(e) = &s.error {
                    tracing::warn!("settings: {e}");
                }
                let _ = app.emit_to("home", "home-state", s);
            }
        })
        .expect("spawn the settings watcher");
}
