//! First run (docs/ux/first-run.md): the system check, the microphone level, the download of
//! every required component with progress, then dictation starts. The page renders what this
//! reports and sends intent.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use utter_pipeline::Settings;
use utter_runtime::Component;

/// Room left on the drive beyond the download, for extraction and logs.
const SPARE_BYTES: u64 = 2_000_000_000;
/// The least GPU memory Utter runs in (the light tier with the speech model, about 6 GB).
const MIN_GPU_BYTES: u64 = 7_500_000_000;
/// The meter's level rate, the Bar's (main.rs): 60 a second.
const LEVEL_EVERY: Duration = Duration::from_millis(16);

/// The required components not installed, or installed with a file missing or cut short.
pub fn missing() -> Vec<&'static Component> {
    utter_runtime::components().iter().filter(|c| !c.optional && !c.is_intact()).collect()
}

/// Which window opens at start (docs/ux/first-run.md, "When Utter starts: three paths").
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Nothing of Utter's is installed yet: the whole first run.
    First,
    /// A new version of a component to fetch: the check and the download only.
    Update,
    /// An installed component is damaged: the same, worded as a repair.
    Repair,
    /// Dictation could not start.
    Problem,
}

impl Mode {
    pub fn query(self) -> &'static str {
        match self {
            Mode::First => "first",
            Mode::Update => "update",
            Mode::Repair => "repair",
            Mode::Problem => "problem",
        }
    }
}

/// Which of the setup paths the missing components call for (`None`: none missing).
pub fn mode_for(missing: &[&Component]) -> Option<Mode> {
    let required: Vec<&Component> = utter_runtime::components().iter().filter(|c| !c.optional).collect();
    if missing.is_empty() {
        None
    } else if required.iter().all(|c| !c.is_installed()) {
        Some(Mode::First)
    } else if missing.iter().any(|c| c.is_installed()) {
        Some(Mode::Repair)
    } else {
        Some(Mode::Update)
    }
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
    crate::start_pipeline(&app);
}

/// Repair: hashes every file that has a checksum, removes each component with a damaged or
/// missing file, then downloads them again as a download does. When every file checks out,
/// the runtime libraries (extracted from archives, so without per-file checksums) are the
/// suspects and are fetched again.
#[tauri::command]
pub fn setup_repair(app: AppHandle, state: tauri::State<'_, Downloading>) {
    if state.0.load(Ordering::Relaxed) {
        return;
    }
    let required: Vec<&Component> = utter_runtime::components().iter().filter(|c| !c.optional).collect();
    let mut damaged: Vec<&Component> =
        required.iter().copied().filter(|c| !c.is_intact() || !c.damaged_files().is_empty()).collect();
    if damaged.is_empty() {
        damaged = required.into_iter().filter(|c| matches!(c.source, utter_runtime::Source::Archive { .. })).collect();
    }
    for c in &damaged {
        tracing::info!(id = %c.id, "repair: fetching again");
        if let Err(e) = c.uninstall() {
            tracing::warn!("repair: {e}");
        }
    }
    setup_download(app, state);
}

/// What a person calls a component.
fn label(id: &str) -> &'static str {
    match id {
        "parakeet-ultra-onnx-fp16" | "silero-vad" => "Speech model",
        "utter-cleanup-e4b" | "gemma4-e4b-qat" | "gemma4-26b-a4b-qat" => "Cleanup model",
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
    /// No NVIDIA GPU at all: nothing to fix here.
    no_gpu: bool,
    download_bytes: u64,
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[tauri::command]
pub fn setup_check() -> Checks {
    let need: u64 = missing().iter().map(|c| c.download_size()).sum();
    let mut checks = Vec::new();
    let gpu = utter_runtime::gpu::probe();
    let no_gpu = gpu.is_err();
    match &gpu {
        Ok(g) => {
            checks.push(Check { id: "gpu", ok: true, value: g.name.clone(), fix: None });
            let ok = g.memory_bytes >= MIN_GPU_BYTES;
            checks.push(Check {
                id: "memory",
                ok,
                value: format!("{:.0} GB", g.memory_bytes as f64 / (1u64 << 30) as f64),
                fix: (!ok).then(|| "Utter needs a graphics card with 8 GB of memory or more.".into()),
            });
            let ok = g.driver_cuda >= utter_runtime::gpu::MIN_DRIVER_CUDA;
            checks.push(Check {
                id: "driver",
                ok,
                value: format!("CUDA {}.{}", g.driver_cuda / 1000, g.driver_cuda % 1000 / 10),
                fix: (!ok).then(|| "Update the NVIDIA driver to version 580 or newer, then check again.".into()),
            });
        }
        Err(e) => {
            tracing::info!("no usable GPU: {e}");
            checks.push(Check { id: "gpu", ok: false, value: "None found".into(), fix: None });
        }
    }
    let dir = utter_core::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    match utter_platform::disk::free_bytes(&dir) {
        Some(free) => {
            let ok = free >= need + SPARE_BYTES;
            checks.push(Check {
                id: "disk",
                ok,
                value: format!("{} free, {} needed", gb(free), gb(need)),
                fix: (!ok)
                    .then(|| format!("Free up {} on the drive with {}.", gb(need + SPARE_BYTES - free), dir.display())),
            });
        }
        None => checks.push(Check { id: "disk", ok: true, value: format!("{} needed", gb(need)), fix: None }),
    }
    Checks { checks, no_gpu, download_bytes: need }
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
        .spawn(move || {
            let todo = missing();
            let total: u64 = todo.iter().map(|c| c.download_size()).sum();
            let emit = |p: DownloadProgress| {
                let _ = app.emit_to("setup", "setup-progress", p);
            };
            let mut before = 0u64;
            for c in todo {
                let label = label(&c.id);
                // Installed but not intact: removed, so it is fetched again.
                if c.is_installed()
                    && let Err(e) = c.uninstall()
                {
                    tracing::warn!("removing the damaged {}: {e}", c.id);
                }
                let result = utter_runtime::install(c, |p| {
                    let (done, stage) = match p {
                        utter_runtime::Progress::Downloading { done, .. } => (before + done, "downloading"),
                        utter_runtime::Progress::Verifying => (before + c.download_size(), "verifying"),
                        utter_runtime::Progress::Extracting => (before + c.download_size(), "extracting"),
                    };
                    emit(DownloadProgress { done, total, stage, label, error: None, problem: None });
                });
                if let Err(e) = result {
                    tracing::error!("installing {}: {e}", c.id);
                    let message = crate::problem::download_message(&e);
                    emit(DownloadProgress {
                        done: before,
                        total,
                        stage: "error",
                        label,
                        error: Some(message),
                        problem: None,
                    });
                    app.state::<Downloading>().0.store(false, Ordering::Relaxed);
                    return;
                }
                before += c.download_size();
            }
            emit(DownloadProgress { done: total, total, stage: "loading", label: "", error: None, problem: None });
            app.state::<Downloading>().0.store(false, Ordering::Relaxed);
            crate::start_pipeline(&app);
        })
        .expect("spawn the download");
}

/// Ends first run: the window closes, Utter stays in the tray.
#[tauri::command]
pub fn setup_finish(app: AppHandle) {
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

fn attention(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("setup")
        && !w.is_focused().unwrap_or(false)
    {
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
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
    let url = format!("setup.html?mode={}", mode.query());
    tauri::WebviewWindowBuilder::new(app, "setup", tauri::WebviewUrl::App(url.into()))
        .title("Utter")
        .inner_size(640.0, 520.0)
        .resizable(false)
        .center()
        .build()
        .map(drop)
}
