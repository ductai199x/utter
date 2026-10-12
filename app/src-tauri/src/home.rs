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
    /// The light model is published (components.json): until then only Quality is offered.
    light_available: bool,
    /// The GPU is big enough for the quality model (utter_pipeline::card_need).
    quality_fits: bool,
    /// What choosing each model would download now, in bytes (0: installed).
    download_bytes: std::collections::HashMap<&'static str, u64>,
    /// The GPU memory each model needs, speech included (utter_pipeline::gpu_need), and the
    /// smallest graphics card it runs on (card_need), in bytes.
    gpu_need: std::collections::HashMap<&'static str, u64>,
    card_need: std::collections::HashMap<&'static str, u64>,
    version: &'static str,
    data_dir: String,
    settings_path: String,
}


/// The GPU's memory, asked once: it does not change while Utter runs.
fn gpu_memory() -> Option<u64> {
    static MEMORY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *MEMORY.get_or_init(|| utter_runtime::gpu::probe().ok().map(|g| g.memory_bytes))
}

/// Bytes `tier` still needs downloaded (its components that are not installed and intact).
/// The models offered, by the name the settings and the page use.
const TIERS: [(&str, utter_llm::cleanup::Tier); 2] =
    [("quality", utter_llm::cleanup::Tier::Quality), ("light", utter_llm::cleanup::Tier::Light)];

fn to_download(tier: utter_llm::cleanup::Tier) -> u64 {
    utter_pipeline::components_for(tier)
        .into_iter()
        .filter_map(|id| utter_runtime::component(id).ok())
        .filter(|c| !c.is_intact())
        .map(|c| c.download_size())
        .sum()
}

fn state() -> State {
    let path = Settings::path();
    let (settings, error) = match Settings::load_or_create(&path) {
        Ok(s) => (Some(s), None),
        Err(e) => (None, Some(e)),
    };
    State {
        settings,
        error,
        microphones: utter_audio::input_devices(),
        light_available: utter_llm::cleanup::Tier::Light.offered(),
        quality_fits: gpu_memory().is_some_and(|m| m >= utter_pipeline::card_need(utter_llm::cleanup::Tier::Quality)),
        download_bytes: TIERS
            .into_iter()
            .map(|(name, tier)| (name, to_download(tier)))
            .collect(),
        gpu_need: TIERS.into_iter().map(|(name, tier)| (name, utter_pipeline::gpu_need(tier))).collect(),
        card_need: TIERS.into_iter().map(|(name, tier)| (name, utter_pipeline::card_need(tier))).collect(),
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
        "logs" => {
            let dir = utter_core::data_dir().join("logs");
            let _ = std::fs::create_dir_all(&dir);
            dir
        }
        _ => return Err(format!("unknown path {which:?}")),
    };
    utter_platform::shell::open(&path).map_err(|e| e.to_string())
}

/// Keeps the sign-in registration in step with the setting. Only an installed copy registers
/// itself: a development build (anything under a cargo `target` folder) never does.
pub fn apply_autostart(settings: &Settings) {
    if utter_core::is_development_build() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if let Err(e) = utter_platform::autostart::set(settings.start_at_login, &exe, crate::splash::AUTOSTART_ARG) {
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
                    // A model whose files are not here yet: setup fetches them first, and
                    // hands the settings over when done (setup_download).
                    let missing = crate::setup::missing();
                    if let Some(mode) = crate::setup::mode_for(&missing) {
                        if app.get_webview_window("setup").is_none()
                            && let Err(e) = crate::setup::open(&app, mode)
                        {
                            tracing::error!("could not open setup: {e}");
                        }
                    } else if let Some(tx) = link.0.lock().expect("pipeline link").as_ref() {
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
