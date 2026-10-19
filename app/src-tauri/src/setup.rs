//! First run (docs/ux/first-run.md): the system check, the microphone level, the download of
//! every required component with progress, then dictation starts. The page renders what this
//! reports and sends intent.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use utter_llm::cleanup::Tier;
use utter_pipeline::Settings;
use utter_pipeline::card::Driver;
use utter_runtime::Component;

/// Room left on the drive beyond the download, for extraction and logs.
const SPARE_BYTES: u64 = 2_000_000_000;
/// The meter's level rate, the Bar's (main.rs): 60 a second.
const LEVEL_EVERY: Duration = Duration::from_millis(16);

/// The required components not installed, or installed with a file missing or cut short.
pub fn missing() -> Vec<&'static Component> {
    required().into_iter().filter(|c| !c.is_intact()).collect()
}

/// The components the chosen tier needs (utter_pipeline::components_for).
fn required() -> Vec<&'static Component> {
    utter_pipeline::components_for(chosen_tier())
        .into_iter()
        .filter_map(|id| utter_runtime::component(id).ok())
        .collect()
}

/// The model in the settings (the default when the file cannot be read), as it runs on this
/// backend: Light where Quality cannot (the processor; utter_pipeline::runtime_choice).
pub fn chosen_model() -> utter_pipeline::settings::Model {
    let model = Settings::load_or_create(&Settings::path()).map(|s| s.model).unwrap_or_default();
    utter_pipeline::runtime_choice::effective_model(model)
}

/// The chosen model's tier, or Quality when that one is not published yet.
pub fn chosen_tier() -> Tier {
    let tier: Tier = chosen_model().into();
    if tier.available() { tier } else { Tier::Quality }
}

/// The model to dictate with while the chosen one downloads: after first run, when the chosen
/// model is not all here but the other one is. Utter starts on it rather than waiting (and the
/// model download opens beside it), as a switch made while running keeps the current model.
pub fn fallback_model() -> Option<utter_pipeline::settings::Model> {
    use utter_pipeline::settings::Model;
    if missing().is_empty() || !setup_complete_marker().exists() {
        return None;
    }
    let chosen = chosen_model();
    [Model::Quality, Model::Light, Model::Mini].into_iter().find(|&m| {
        let tier: Tier = m.into();
        m != chosen
            && tier.offered()
            // Only what runs here: the pipeline would run another model, which is missing.
            && utter_pipeline::runtime_choice::runs_on(m, utter_runtime::backend())
            && utter_pipeline::components_for(tier)
                .into_iter()
                .all(|id| utter_runtime::component(id).is_ok_and(|c| c.is_intact()))
    })
}

/// Which window opens at start (docs/ux/first-run.md, "When Utter starts: three paths").
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// First run has not finished (nothing installed yet, or it was interrupted): the whole
    /// first run.
    First,
    /// A new version of a component to fetch: the check and the download only.
    Update,
    /// The model chosen in Settings is not downloaded yet: the same, worded as that.
    Model,
    /// What the runtime chosen in Settings runs on is not downloaded yet (or a new component
    /// it needs): the same, worded as that; nothing runs until it is here.
    Runtime,
    /// An installed component is damaged: the same, worded as a repair.
    Repair,
    /// Dictation could not start.
    Problem,
    /// Everything is installed but the person never finished or skipped first run's Try it
    /// (first run was interrupted after the download): Try it alone, so they learn the key.
    Try,
}

impl Mode {
    pub fn query(self) -> &'static str {
        match self {
            Mode::First => "first",
            Mode::Update => "update",
            Mode::Model => "model",
            Mode::Runtime => "runtime",
            Mode::Repair => "repair",
            Mode::Problem => "problem",
            Mode::Try => "try",
        }
    }
}

/// Written into the data folder once everything first run downloads is installed. Without it
/// a first run that was interrupted comes back as first run, not as an update.
fn setup_complete_marker() -> std::path::PathBuf {
    utter_core::data_dir().join("setup-complete")
}

/// Records that setup has everything it needs (at start with nothing missing, and when a
/// download finishes).
pub fn mark_setup_complete() {
    let path = setup_complete_marker();
    if !path.exists()
        && let Err(e) = std::fs::write(&path, "")
    {
        tracing::warn!("could not write {}: {e}", path.display());
    }
}

/// Written when the person finishes or skips first run's Try it.
fn tried_marker() -> std::path::PathBuf {
    utter_core::data_dir().join("tried")
}

/// Whether the person has been through Try it: finished or skipped it, or dictated already
/// (an install from before the marker).
pub fn tried() -> bool {
    tried_marker().exists()
        || std::fs::metadata(utter_core::data_dir().join("history.jsonl")).is_ok_and(|m| m.len() > 0)
}

/// Which of the setup paths the missing components call for (`None`: none missing).
pub fn mode_for(missing: &[&Component]) -> Option<Mode> {
    let required = required();
    if missing.is_empty() {
        None
    } else if !setup_complete_marker().exists() || required.iter().all(|c| !c.is_installed()) {
        Some(Mode::First)
    } else if missing.iter().all(|c| !c.is_installed() && older_version_installed(c)) {
        // Every missing one replaces a version that is here: an update.
        Some(Mode::Update)
    } else if missing.iter().all(|c| !c.is_installed() && !older_version_installed(c)) {
        // None was ever here: another runtime's components, chosen in Settings.
        Some(Mode::Runtime)
    } else {
        // Damaged, or gone with nothing it replaces.
        Some(Mode::Repair)
    }
}

/// Whether another version of `c` is installed (an update then replaces it).
fn older_version_installed(c: &Component) -> bool {
    let current = c.dir();
    let prefix = format!("{}-", c.id);
    std::fs::read_dir(utter_runtime::runtime_dir()).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let path = e.path();
            path != current
                && e.file_name().to_string_lossy().starts_with(&prefix)
                && path.join("utter-component.json").is_file()
        })
    })
}

/// One tier as first run's "Choose your model" step shows it (docs/ux/first-run.md).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCard {
    /// The settings value: "quality" or "light".
    id: &'static str,
    /// GPU memory the tier needs, in bytes.
    gpu_bytes: u64,
    /// The graphics card memory it is offered on, in bytes.
    card_bytes: u64,
    /// What choosing it downloads now (0: installed).
    download_bytes: u64,
    /// This GPU has the memory for it, and it runs on this backend.
    fits: bool,
    /// Why not, when it does not fit.
    why: Option<String>,
    /// Everything runs on the processor: the cards speak of memory, not GPU memory.
    on_processor: bool,
    recommended: bool,
    selected: bool,
}

/// The tiers on offer, Quality first, with the best fit for this GPU recommended: Quality when
/// the card has the memory for it (or Light is not published), else Light.
#[tauri::command]
pub fn setup_models() -> Vec<ModelCard> {
    use utter_pipeline::settings::Model;
    let memory = utter_pipeline::card::graphics_card().map(|g| g.memory_bytes).unwrap_or(0);
    let backend = utter_runtime::backend();
    let on_processor = backend == utter_runtime::Backend::Cpu;
    let runs = |tier: Tier| utter_pipeline::runtime_choice::runs_on(tier.into(), backend);
    let fits = |tier: Tier| runs(tier) && memory >= utter_pipeline::card_need(tier);
    // The best that fits; on the processor the fastest (latency first), Extra light.
    let recommended = if on_processor && Tier::Mini.offered() {
        Tier::Mini
    } else if fits(Tier::Quality) || !Tier::Light.available() {
        Tier::Quality
    } else {
        Tier::Light
    };
    let chosen = chosen_tier();
    [(Model::Quality, "quality"), (Model::Light, "light"), (Model::Mini, "mini")]
        .into_iter()
        .map(|(model, id)| (Tier::from(model), id))
        // Quality shows grayed on the processor (it needs a graphics card); Extra light shows
        // only there (with a graphics card Light is as fast).
        .filter(|&(tier, _)| tier.offered() && (tier != Tier::Mini || on_processor))
        .map(|(tier, id)| ModelCard {
            id,
            gpu_bytes: utter_pipeline::gpu_need(tier),
            card_bytes: utter_pipeline::card_need(tier),
            download_bytes: utter_pipeline::components_for(tier)
                .into_iter()
                .filter_map(|c| utter_runtime::component(c).ok())
                .filter(|c| !c.is_intact())
                .map(|c| c.download_size())
                .sum(),
            fits: fits(tier),
            why: (!fits(tier)).then(|| {
                if !runs(tier) {
                    "Needs a graphics card".to_owned()
                } else {
                    format!("Needs a {} GB graphics card", utter_pipeline::card_need(tier).div_ceil(1 << 30))
                }
            }),
            on_processor,
            recommended: tier == recommended,
            selected: tier == chosen,
        })
        .collect()
}

/// Saves the tier first run's model step chose.
#[tauri::command]
pub fn setup_choose(model: utter_pipeline::settings::Model) -> Result<(), String> {
    let path = Settings::path();
    let mut s = Settings::load_or_create(&path)?;
    s.model = model;
    s.save(&path)
}

/// The last reason dictation could not start, for the problem screen.
#[derive(Default)]
pub struct LastProblem(pub Mutex<Option<crate::problem::Problem>>);

#[tauri::command]
pub fn setup_problem(state: tauri::State<'_, LastProblem>) -> Option<crate::problem::Problem> {
    state.0.lock().expect("problem lock").clone()
}

/// Tries to start dictation again (the problem screen's Retry).
#[tauri::command]
pub fn setup_retry(app: AppHandle) {
    if !crate::start_pipeline(&app) {
        // Running already: a model switch failed. Its failure is forgotten (it would keep the
        // running model in the settings sent, home::Models::for_pipeline), and it is tried again.
        app.state::<crate::home::Models>().clear_failed();
        crate::reapply_settings(&app);
    }
}

/// Repair: hashes every file that has a checksum, removes each component with a damaged or
/// missing file, then downloads them again as a download does. When every file checks out,
/// the runtime libraries (extracted from archives, so without per-file checksums) are the
/// suspects and are fetched again.
///
/// Hashing takes seconds, so it runs on the download's thread, never the window's. A component
/// that cannot be removed is in use (after a failed start its libraries stay loaded): Utter
/// restarts and removes it before anything loads ([`finish_pending_repair`]).
#[tauri::command]
pub fn setup_repair(app: AppHandle, state: tauri::State<'_, Downloading>) {
    if state.0.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::Builder::new()
        .name("utter-setup-repair".into())
        .spawn(move || {
            let _ = app.emit_to("setup", "setup-progress", DownloadProgress::stage("verifying"));
            let required = required();
            let mut damaged: Vec<&Component> =
                required.iter().copied().filter(|c| !c.is_intact() || !c.damaged_files().is_empty()).collect();
            if damaged.is_empty() {
                damaged = required
                    .into_iter()
                    .filter(|c| matches!(c.source, utter_runtime::Source::Archive { .. }))
                    .collect();
            }
            tracing::info!(ids = ?damaged.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), "repair: fetching again");
            remove_or_restart(&app, &damaged);
            download(&app);
        })
        .expect("spawn the repair");
}

/// Lists the components to remove at the next start.
fn pending_repair_file() -> std::path::PathBuf {
    utter_core::data_dir().join("repair-pending")
}

/// Removes `components`. One in use cannot be: the ids are written down for the next start,
/// and Utter restarts (this does not return then).
fn remove_or_restart(app: &AppHandle, components: &[&Component]) {
    let stuck: Vec<&str> = components
        .iter()
        .filter(|c| match c.uninstall() {
            Ok(()) => false,
            Err(e) => {
                tracing::warn!("{} is in use ({e}); removing it after a restart", c.id);
                true
            }
        })
        .map(|c| c.id.as_str())
        .collect();
    if stuck.is_empty() {
        return;
    }
    if let Err(e) = std::fs::write(pending_repair_file(), stuck.join("\n")) {
        tracing::error!("could not write {}: {e}", pending_repair_file().display());
        return;
    }
    app.restart();
}

/// At start, before anything loads: removes what a repair could not (the components were in
/// use then). Returns whether a repair was pending, so setup opens worded as one.
pub fn finish_pending_repair() -> bool {
    let path = pending_repair_file();
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    for id in text.lines().filter(|l| !l.trim().is_empty()) {
        match utter_runtime::component(id.trim()) {
            Ok(c) => match c.uninstall() {
                Ok(()) => tracing::info!(id, "repair: removed after the restart"),
                Err(e) => tracing::error!("repair: could not remove {id}: {e}"),
            },
            Err(e) => tracing::warn!("repair: {e}"),
        }
    }
    // Removed even if a removal failed: a file that cannot go must not restart Utter forever.
    let _ = std::fs::remove_file(&path);
    true
}

/// What a person calls a component.
fn label(id: &str) -> &'static str {
    match id {
        id if id.starts_with("parakeet-") || id == "silero-vad" => "Speech model",
        id if id.starts_with("utter-") || id.starts_with("gemma") => "Cleanup model",
        _ if utter_runtime::backend() == utter_runtime::Backend::Cpu => "Processor runtime",
        _ => "GPU runtime",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    id: &'static str,
    ok: bool,
    value: String,
    /// What to do about it, when it fails.
    fix: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checks {
    checks: Vec<Check>,
    /// No graphics card at all: nothing to fix here.
    no_gpu: bool,
    download_bytes: u64,
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// What the keyboard check and problem ask for (problem.rs too).
pub const KEYBOARD_FIX: &str = if cfg!(target_os = "macos") {
    "Utter needs Input Monitoring, to hear the dictation key, and Accessibility, to paste. Allow opens System Settings: turn Utter on in both, then open Utter again."
} else {
    "Utter needs this to hear the dictation key and paste for you. Allow asks for your password once."
};

#[tauri::command]
pub fn setup_check(before_choice: bool) -> Checks {
    let gpu = utter_pipeline::card::graphics_card();
    // Before first run's model step the person can still pick the smaller model: the disk
    // must hold the smallest download on offer (the model step checks the chosen one).
    let need: u64 = if before_choice {
        [Tier::Quality, Tier::Light, Tier::Mini]
            .into_iter()
            .filter(|&t| t.offered() && utter_pipeline::runtime_choice::runs_on(t.into(), utter_runtime::backend()))
            .map(|t| {
                utter_pipeline::components_for(t)
                    .into_iter()
                    .filter_map(|id| utter_runtime::component(id).ok())
                    .filter(|c| !c.is_intact())
                    .map(|c| c.download_size())
                    .sum::<u64>()
            })
            .min()
            .unwrap_or(0)
    } else {
        missing().iter().map(|c| c.download_size()).sum()
    };
    let mut checks = Vec::new();
    let no_gpu = gpu.is_err();
    match &gpu {
        Ok(g) => {
            if utter_runtime::backend() == utter_runtime::Backend::Cpu {
                // No graphics card to run on (or the processor chosen in Settings): said plainly,
                // with what it means.
                let only = utter_pipeline::runtime_choice::available().len() == 1;
                checks.push(Check {
                    id: "processor",
                    ok: true,
                    value: if only { "The processor" } else { "The processor (your choice)" }.into(),
                    fix: only.then(|| {
                        "No graphics card found: Utter runs on the processor, with its smaller models.".into()
                    }),
                });
            } else {
                checks.push(Check { id: "gpu", ok: true, value: g.name.clone(), fix: None });
            }
            // Enough for the smallest tier on offer: which one runs is the next step's choice.
            let smallest = utter_pipeline::card_need(if Tier::Light.available() { Tier::Light } else { Tier::Quality });
            let ok = g.memory_bytes >= smallest;
            let gib = g.memory_bytes as f64 / (1u64 << 30) as f64;
            // The processor and Apple Silicon run on the system's memory: no graphics card to ask
            // for, and no driver of its own to check.
            let shared = g.driver == Driver::System;
            checks.push(Check {
                id: if shared { "shared-memory" } else { "memory" },
                ok,
                value: format!("{gib:.0} GB"),
                fix: (!ok).then(|| {
                    let need = smallest.div_ceil(1 << 30);
                    if shared {
                        format!("Utter needs {need} GB of memory or more.")
                    } else {
                        format!("Utter needs a graphics card with {need} GB of memory or more.")
                    }
                }),
            });
            if shared {
                // Nothing more to check about the processor.
            } else {
                let ok = g.driver.ok();
                let (value, fix) = match g.driver {
                    Driver::Cuda(v) => (
                        format!("CUDA {}.{}", v / 1000, v % 1000 / 10),
                        "Update the NVIDIA driver to version 580 or newer, then check again.",
                    ),
                    Driver::Vulkan(present) => (
                        if present { "Vulkan" } else { "No Vulkan" }.to_owned(),
                        "Update the graphics driver from your card maker's site, then check again.",
                    ),
                    Driver::System => ("Built in".to_owned(), ""),
                };
                checks.push(Check { id: "driver", ok, value, fix: (!ok).then(|| fix.into()) });
            }
        }
        Err(e) => {
            tracing::info!("no usable GPU: {e}");
            checks.push(Check { id: "gpu", ok: false, value: "None found".into(), fix: None });
        }
    }
    // Linux: a udev rule; macOS: the Input Monitoring and Accessibility permissions.
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        let access = utter_platform::hotkey::input_access();
        if let Err(e) = &access {
            tracing::info!("keyboard access: {e}");
        }
        let ok = access.is_ok();
        checks.push(Check {
            id: "keyboard",
            ok,
            value: if ok { "Allowed" } else { "Not allowed" }.into(),
            fix: (!ok).then(|| KEYBOARD_FIX.into()),
        });
    }
    let dir = utter_core::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    match utter_platform::disk::free_bytes(&dir) {
        Some(free) => {
            // "Needed" is the whole room the install takes, so it and "Free up" agree.
            let room = need + SPARE_BYTES;
            let ok = free >= room;
            checks.push(Check {
                id: "disk",
                ok,
                value: format!("{} free, {} needed", gb(free), gb(room)),
                fix: (!ok).then(|| format!("Free up {} on the drive with {}.", gb(room - free), dir.display())),
            });
        }
        None => {
            checks.push(Check { id: "disk", ok: true, value: format!("{} needed", gb(need + SPARE_BYTES)), fix: None })
        }
    }
    Checks { checks, no_gpu, download_bytes: need }
}

/// "Allow" on the keyboard check (Linux): installs the udev rule through the desktop's password
/// dialog. Async, so the UI thread never waits on the person answering it.
#[tauri::command]
pub async fn setup_allow_input() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(utter_platform::hotkey::allow_input).await.map_err(|e| e.to_string())?
}

/// The microphone open for the level check while step 2 shows, and which feed of its level is
/// current: starting or stopping bumps it, and an older feed ends.
#[derive(Default)]
pub struct MicCheck(Mutex<Option<utter_audio::Microphone>>, Arc<AtomicU64>);

#[tauri::command]
pub fn setup_microphones() -> Vec<String> {
    utter_audio::input_devices()
}

/// Opens the microphone (empty: the default), saves the choice, and streams its level to the
/// page as "setup-level" until [`setup_mic_stop`].
#[tauri::command]
pub fn setup_mic_start(app: AppHandle, state: tauri::State<'_, MicCheck>, device: String) -> Result<(), String> {
    let cfg = utter_audio::MicConfig { device: (!device.is_empty()).then(|| device.clone()), ..Default::default() };
    let mic = utter_audio::Microphone::open(&cfg).map_err(|e| e.to_string())?;
    let meter = mic.level_meter();
    *state.0.lock().expect("mic check lock") = Some(mic);
    let path = Settings::path();
    if let Ok(mut s) = Settings::load_or_create(&path) {
        s.microphone = device;
        let _ = s.save(&path);
    }
    let current = state.1.clone();
    let mine = current.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        while current.load(Ordering::Relaxed) == mine {
            let _ = app.emit_to("setup", "setup-level", meter.level());
            std::thread::sleep(LEVEL_EVERY);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn setup_mic_stop(state: tauri::State<'_, MicCheck>) {
    state.1.fetch_add(1, Ordering::Relaxed);
    *state.0.lock().expect("mic check lock") = None;
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    done: u64,
    total: u64,
    /// "downloading", "verifying", "extracting", "loading", "ready", "error" (a download) or
    /// "start-error" (dictation could not start: `problem` says why).
    stage: &'static str,
    label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<crate::problem::Problem>,
}

impl DownloadProgress {
    /// A stage with no bytes to show yet.
    fn stage(stage: &'static str) -> Self {
        Self { done: 0, total: 0, stage, label: "", error: None, problem: None }
    }
}

/// Whether a download is running, so a second click does not start another.
#[derive(Default)]
pub struct Downloading(AtomicBool);

/// Installs every missing component, reporting "setup-progress", then starts dictation.
#[tauri::command]
pub fn setup_download(app: AppHandle, state: tauri::State<'_, Downloading>) {
    if state.0.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::Builder::new()
        .name("utter-setup-download".into())
        .spawn(move || download(&app))
        .expect("spawn the download");
}

/// The download itself, on its own thread, with [`Downloading`] set; clears it when done.
fn download(app: &AppHandle) {
    let todo = missing();
    let total: u64 = todo.iter().map(|c| c.download_size()).sum();
    let emit = |p: DownloadProgress| {
        let _ = app.emit_to("setup", "setup-progress", p);
    };
    // Installed but not intact: removed first, so they are fetched again.
    let damaged: Vec<&Component> = todo.iter().copied().filter(|c| c.is_installed()).collect();
    remove_or_restart(app, &damaged);
    let mut before = 0u64;
    for c in todo {
        let label = label(&c.id);
        // Bytes of this component downloaded so far. Verifying follows each file, so it
        // holds the bar there: a component of several files would otherwise jump to
        // its end after the first and fall back when the next one starts.
        let mut got = 0u64;
        let result = utter_runtime::install(c, |p| {
            let (done, stage) = match p {
                utter_runtime::Progress::Downloading { done, .. } => {
                    got = done;
                    (before + done, "downloading")
                }
                utter_runtime::Progress::Verifying => (before + got, "verifying"),
                utter_runtime::Progress::Extracting => (before + c.download_size(), "extracting"),
            };
            emit(DownloadProgress { done, total, stage, label, error: None, problem: None });
        });
        if let Err(e) = result {
            tracing::error!("installing {}: {e}", c.id);
            let message = crate::problem::download_message(&e);
            emit(DownloadProgress { done: before, total, stage: "error", label, error: Some(message), problem: None });
            app.state::<Downloading>().0.store(false, Ordering::Relaxed);
            return;
        }
        before += c.download_size();
    }
    mark_setup_complete();
    emit(DownloadProgress { done: total, total, stage: "loading", label: "", error: None, problem: None });
    app.state::<Downloading>().0.store(false, Ordering::Relaxed);
    // Dictation not running yet: it starts, and says when it is ready. Already running
    // (a model change from Home needed the download, or an update downloaded while the
    // earlier version ran): it takes the settings again and loads what was downloaded, and
    // the switch reports back (main.rs, Event::Applied). Running on the chosen model's
    // current files already (the person chose it back meanwhile): nothing to switch, so
    // nothing would report back; setup is done.
    if !crate::start_pipeline(app) {
        let models = app.state::<crate::home::Models>();
        if models.running() == Some(chosen_model()) && !models.on_earlier() {
            ready(app);
        } else {
            crate::reapply_settings(app);
        }
    }
}

/// Whether dictation runs (Try it alone waits for it: it may still be loading).
#[tauri::command]
pub fn setup_dictation_ready(app: AppHandle) -> bool {
    app.state::<crate::home::Models>().running().is_some()
}

/// Ends first run: the window closes, Utter stays in the tray. `tried`: from Try it (Finish
/// or Skip), which then never opens again.
#[tauri::command]
pub fn setup_finish(app: AppHandle, tried: Option<bool>) {
    if tried == Some(true)
        && let Err(e) = std::fs::write(tried_marker(), "")
    {
        tracing::warn!("could not write {}: {e}", tried_marker().display());
    }
    if let Some(w) = app.get_webview_window("setup") {
        let _ = w.destroy();
    }
}

/// Tells the page dictation is ready (step 4 can start).
pub fn ready(app: &AppHandle) {
    let p = DownloadProgress { done: 0, total: 0, stage: "ready", label: "", error: None, problem: None };
    let _ = app.emit_to("setup", "setup-progress", p);
    // The download took minutes and the person may be elsewhere: the taskbar button flashes.
    attention(app);
}

/// On the UI thread, without waiting for it: asking a window whether it is focused from another
/// thread blocks until the UI thread answers, and dictation calls this.
fn attention(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = handle.get_webview_window("setup")
            && !w.is_focused().unwrap_or(false)
        {
            let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
    });
}

/// Dictation could not start: the setup window shows why, opening in problem mode if it is not
/// open already (a start with everything installed).
pub fn failed(app: &AppHandle, problem: crate::problem::Problem) {
    *app.state::<LastProblem>().0.lock().expect("problem lock") = Some(problem.clone());
    if app.get_webview_window("setup").is_some() {
        let p = DownloadProgress {
            done: 0,
            total: 0,
            stage: "start-error",
            label: "",
            error: Some(problem.message.clone()),
            problem: Some(problem),
        };
        let _ = app.emit_to("setup", "setup-progress", p);
        attention(app);
    } else if let Err(e) = open(app, Mode::Problem) {
        tracing::error!("could not open the problem window: {e}");
    }
}

/// Opens the setup window in `mode`.
pub fn open(app: &AppHandle, mode: Mode) -> tauri::Result<()> {
    open_with(app, mode, false)
}

/// [`open`]; `carry_on`: the person already asked for this (a repair that restarted Utter), so
/// the download starts once the system check passes.
/// The setup window's height: Linux and macOS check keyboard access too, and its row (with why,
/// when it is missing) did not fit in Windows' 520 px; it scrolled out of sight.
const SETUP_HEIGHT: f64 = if cfg!(windows) { 520.0 } else { 600.0 };

pub fn open_with(app: &AppHandle, mode: Mode, carry_on: bool) -> tauri::Result<()> {
    let url = format!("setup.html?mode={}{}", mode.query(), if carry_on { "&go=1" } else { "" });
    tauri::WebviewWindowBuilder::new(app, "setup", tauri::WebviewUrl::App(url.into()))
        .title("Utter")
        .inner_size(640.0, SETUP_HEIGHT)
        .resizable(false)
        .center()
        .build()
        .map(drop)
}
