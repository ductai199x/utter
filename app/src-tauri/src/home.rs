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
use utter_pipeline::settings::{CpuThreads, DictationKey, Model, clean_terms};

/// How often the watcher looks at the file's modification time.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Where the watcher sends settings once the pipeline runs (it may start later, or not at all).
#[derive(Clone, Default)]
pub struct PipelineLink(pub Arc<Mutex<Option<Sender<Settings>>>>);

/// The model dictation runs on, and one it failed to switch to (managed state).
#[derive(Default)]
pub struct Models {
    running: Mutex<Option<Model>>,
    failed: Mutex<Option<Model>>,
    /// The running model's components are earlier versions (it started during an update).
    on_earlier: Mutex<bool>,
}

impl Models {
    pub fn running(&self) -> Option<Model> {
        *self.running.lock().expect("models lock")
    }

    /// Dictation started on `model`, or switched to it.
    pub fn set_running(&self, model: Model) {
        *self.running.lock().expect("models lock") = Some(model);
        *self.failed.lock().expect("models lock") = None;
        *self.on_earlier.lock().expect("models lock") = false;
    }

    /// What runs started while an update downloads, on the earlier versions.
    pub fn set_on_earlier(&self) {
        *self.on_earlier.lock().expect("models lock") = true;
    }

    pub fn on_earlier(&self) -> bool {
        *self.on_earlier.lock().expect("models lock")
    }

    /// Dictation stopped: nothing runs until it starts again.
    pub fn stopped(&self) {
        *self.running.lock().expect("models lock") = None;
    }

    /// A switch to `model` failed: it is not tried again until the person chooses a model again.
    pub fn set_failed(&self, model: Model) {
        *self.failed.lock().expect("models lock") = Some(model);
    }

    pub fn clear_failed(&self) {
        *self.failed.lock().expect("models lock") = None;
    }

    /// The settings to hand the running pipeline: as they are, except that the model stays the
    /// running one while the chosen one is not downloaded yet or just failed to load. Everything
    /// else (dictionary, paste rules) still applies at once.
    pub fn for_pipeline(&self, mut settings: Settings) -> Settings {
        let Some(running) = self.running() else { return settings };
        let chosen = settings.model;
        let failed = *self.failed.lock().expect("models lock") == Some(chosen);
        if chosen != running && (failed || !crate::setup::missing().is_empty()) {
            settings.model = running;
        }
        settings
    }
}

/// What the page needs to draw: the settings, or why the file cannot be read.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct State {
    settings: Option<Settings>,
    error: Option<String>,
    microphones: Vec<String>,
    /// The light model is published (components.json): until then only Quality is offered.
    light_available: bool,
    /// The models that run on this backend and are published, best first.
    models: Vec<Model>,
    /// The GPU is big enough for the quality model (utter_pipeline::card_need), and Utter runs
    /// on a graphics card at all (not the processor).
    quality_fits: bool,
    /// What runs the models now ("cuda", "vulkan", "metal" or "cpu").
    running: utter_pipeline::settings::Runtime,
    /// The runtimes this computer can run, best first, for Settings.
    runtimes: Vec<RuntimeOption>,
    /// The processor threads setting Utter started with (it applies at start).
    running_threads: CpuThreads,
    /// The threads each processor threads setting gives the models on this computer.
    thread_counts: std::collections::HashMap<&'static str, usize>,
    /// What choosing each model would download now, in bytes (0: installed).
    download_bytes: std::collections::HashMap<&'static str, u64>,
    /// The GPU memory each model needs, speech included (utter_pipeline::gpu_need), and the
    /// smallest graphics card it runs on (card_need), in bytes.
    gpu_need: std::collections::HashMap<&'static str, u64>,
    card_need: std::collections::HashMap<&'static str, u64>,
    /// The keys the person can dictate with.
    dictation_keys: Vec<KeyOption>,
    version: &'static str,
    data_dir: String,
    settings_path: String,
    /// "windows", "linux" or "macos": for the words that differ (Start with Windows, Cmd+V).
    os: &'static str,
    /// The desktop shows tray icons; without them, Settings offers Quit.
    tray: bool,
}

/// A runtime as Settings lists it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RuntimeOption {
    id: utter_pipeline::settings::Runtime,
    name: &'static str,
    detail: &'static str,
    /// What switching to it downloads, for the model it would run (0: all here).
    download_bytes: u64,
}

/// The runtimes this computer can run, with what each would download.
fn runtimes() -> Vec<RuntimeOption> {
    use utter_runtime::Backend;
    let model = Settings::load_or_create(&Settings::path()).map(|s| s.model).unwrap_or_default();
    utter_pipeline::runtime_choice::available()
        .iter()
        .map(|&backend| {
            let (name, detail) = match backend {
                Backend::Cuda => ("NVIDIA CUDA", "Fastest on NVIDIA graphics cards."),
                Backend::Vulkan => ("Vulkan", "Any graphics card: AMD, Intel or NVIDIA."),
                Backend::Metal => ("Apple GPU", "The Mac's own graphics."),
                Backend::Cpu => ("Processor", "No graphics card needed. Slower; Light and Extra light only."),
            };
            let runs = if utter_pipeline::runtime_choice::runs_on(model, backend) { model } else { Model::Light };
            let download_bytes = utter_pipeline::components_on(runs.into(), backend)
                .into_iter()
                .filter_map(|id| utter_runtime::component(id).ok())
                .filter(|c| !c.is_intact())
                .map(|c| c.download_size())
                .sum();
            RuntimeOption { id: utter_pipeline::runtime_choice::setting_for(backend), name, detail, download_bytes }
        })
        .collect()
}

/// A dictation key as Settings lists it: its name in settings.toml and as people read it.
#[derive(Serialize, Clone)]
struct KeyOption {
    id: String,
    name: String,
}

/// The GPU's memory, asked once: it does not change while Utter runs.
fn gpu_memory() -> Option<u64> {
    static MEMORY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *MEMORY.get_or_init(|| utter_pipeline::card::graphics_card().ok().map(|g| g.memory_bytes))
}

/// Bytes `tier` still needs downloaded (its components that are not installed and intact).
/// The models offered, by the name the settings and the page use.
const TIERS: [(&str, utter_llm::cleanup::Tier); 3] = [
    ("quality", utter_llm::cleanup::Tier::Quality),
    ("light", utter_llm::cleanup::Tier::Light),
    ("mini", utter_llm::cleanup::Tier::Mini),
];

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
        models: utter_pipeline::runtime_choice::models_on(utter_runtime::backend())
            .into_iter()
            .filter(|&m| utter_llm::cleanup::Tier::from(m).offered())
            .collect(),
        quality_fits: utter_runtime::backend() != utter_runtime::Backend::Cpu
            && gpu_memory().is_some_and(|m| m >= utter_pipeline::card_need(utter_llm::cleanup::Tier::Quality)),
        running: utter_pipeline::runtime_choice::setting_for(utter_runtime::backend()),
        runtimes: runtimes(),
        running_threads: *STARTED_THREADS.get().unwrap_or(&CpuThreads::Default),
        thread_counts: [CpuThreads::Default, CpuThreads::High]
            .into_iter()
            .map(|t| {
                (if t == CpuThreads::High { "high" } else { "default" }, utter_platform::cpu::threads_at(t.percent()))
            })
            .collect(),
        download_bytes: TIERS.into_iter().map(|(name, tier)| (name, to_download(tier))).collect(),
        gpu_need: TIERS.into_iter().map(|(name, tier)| (name, utter_pipeline::gpu_need(tier))).collect(),
        card_need: TIERS.into_iter().map(|(name, tier)| (name, utter_pipeline::card_need(tier))).collect(),
        dictation_keys: DictationKey::offered()
            .iter()
            .map(|k| KeyOption { id: utter_pipeline::settings::word(k), name: k.name() })
            .collect(),
        version: env!("CARGO_PKG_VERSION"),
        data_dir: utter_core::data_dir().display().to_string(),
        settings_path: path.display().to_string(),
        os: std::env::consts::OS,
        tray: utter_platform::shell::tray_shown(),
    }
}

/// The processor threads setting Utter started with (set once, in main).
static STARTED_THREADS: std::sync::OnceLock<CpuThreads> = std::sync::OnceLock::new();

pub fn set_started_threads(threads: CpuThreads) {
    let _ = STARTED_THREADS.set(threads);
}

/// Restart, to switch the runtime or the processor threads Settings saved (both apply at start).
#[tauri::command]
pub fn home_restart(app: tauri::AppHandle) {
    tracing::info!("restarting to apply the runtime or processor threads");
    app.restart();
}

/// Quit, from Settings on a desktop without tray icons (the tray's Quit elsewhere).
#[tauri::command]
pub fn home_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn home_state() -> State {
    state()
}

/// The name of the key held to dictate ("Right Ctrl"), for the pages that tell people to hold it.
#[tauri::command]
pub fn dictation_key() -> String {
    dictation_key_name()
}

/// [`dictation_key`], for Rust: the saved key, or the default when the file cannot be read.
pub fn dictation_key_name() -> String {
    Settings::load_or_create(&Settings::path()).map(|s| s.dictation_key).unwrap_or_default().name()
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
            let dir = utter_core::logs::dir();
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

/// Turns Utter's logs on or off with the setting; off deletes them (utter_core::logs).
pub fn apply_logs(settings: &Settings) {
    if utter_core::logs::enabled() != settings.keep_logs {
        utter_core::logs::set_enabled(settings.keep_logs);
        // Off: this line is not written, and the writer lets go of the file.
        tracing::info!(on = settings.keep_logs, "logs");
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

/// Opens the Home window for a second start of Utter. On Linux the window manager raises a
/// window only for the launch that asked (its startup token); without it GNOME leaves Home
/// behind the focused window and shows "Utter is ready" instead. The second start's token
/// reaches here through a file (keep_launch_token): the single-instance plugin passes only
/// arguments, and GTK takes the variable out of the environment as it starts.
pub fn show_launched(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    if let (Some(token), Some(w)) = (take_launch_token(), app.get_webview_window("home")) {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(gtk_window) = w.gtk_window() {
                use gtk::prelude::GtkWindowExt;
                gtk_window.set_startup_id(&token);
            }
            show(&handle);
        });
        return;
    }
    show(app);
}

/// Where a start of Utter leaves its launch token for the running one.
#[cfg(target_os = "linux")]
fn launch_token_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("utter-launch-token"))
}

/// At the top of main, before GTK reads it: this start's launch token, for the running Utter
/// should this start find one (show_launched), else none, so a stale token is never used.
/// The windows are X11 ones (GDK_BACKEND), whose token is DESKTOP_STARTUP_ID; launchers set it
/// and XDG_ACTIVATION_TOKEN to the same value.
#[cfg(target_os = "linux")]
pub fn keep_launch_token() {
    let Some(path) = launch_token_path() else { return };
    let token = ["DESKTOP_STARTUP_ID", "XDG_ACTIVATION_TOKEN"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()));
    match token {
        Some(token) => {
            let _ = std::fs::write(&path, token);
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(target_os = "linux")]
fn take_launch_token() -> Option<String> {
    let path = launch_token_path()?;
    let token = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(token.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Applies `settings.toml` on every change: to the pipeline, and to the page.
pub fn watch(app: AppHandle, link: PipelineLink) {
    std::thread::Builder::new()
        .name("utter-settings-watch".into())
        .spawn(move || {
            let path = Settings::path();
            let mut seen: Option<SystemTime> = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let mut chosen: Option<Model> = None;
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
                    apply_logs(settings);
                    let models = app.state::<Models>();
                    // A model chosen again may be tried again, even one that failed.
                    if chosen.is_some_and(|m| m != settings.model) {
                        models.clear_failed();
                    }
                    chosen = Some(settings.model);
                    // Before dictation runs, startup (first run, or the problem screen) decides
                    // what happens; the settings reach the pipeline when it starts.
                    if let Some(running) = models.running() {
                        // components.json cannot change while Utter runs: what a settings change
                        // makes missing is a newly chosen model. Setup fetches it, and hands the
                        // model over when done (setup_download); the rest applies now. A file of
                        // the running model gone missing is for the next start to find.
                        if settings.model != running
                            && !crate::setup::missing().is_empty()
                            && app.get_webview_window("setup").is_none()
                            && let Err(e) = crate::setup::open(&app, crate::setup::Mode::Model)
                        {
                            tracing::error!("could not open setup: {e}");
                        }
                        if let Some(tx) = link.0.lock().expect("pipeline link").as_ref() {
                            let _ = tx.send(models.for_pipeline(settings.clone()));
                        }
                    }
                }
                if let Some(e) = &s.error {
                    tracing::warn!("settings: {e}");
                }
                // The Paste rules window shows the same settings.
                let _ = app.emit_to("rules", "home-state", s.clone());
                let _ = app.emit_to("home", "home-state", s);
            }
        })
        .expect("spawn the settings watcher");
}
