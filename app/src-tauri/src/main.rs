//! Utter's app: the Bar overlay and the tray, driven by the dictation pipeline.
//!
//! The Bar window is created hidden with its page already loaded, so showing it costs one
//! frame. It never takes focus and lets every click through (docs/ux/bar.md).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use utter_pipeline::{Event, Pipeline, PipelineConfig, Settings};
use utter_platform::overlay;

mod args;
mod history;
mod home;
mod problem;
mod setup;
mod splash;

/// The Bar window's size in logical pixels (tauri.conf.json): room for the largest pill plus
/// its shadow.
const BAR_SIZE: (u32, u32) = (576, 168);
/// The pill sits 12 px above the window's bottom edge, and 48 px above the work area's.
const BAR_BOTTOM_MARGIN: u32 = 48 - 12;
/// How long the Done check shows before the Bar leaves.
const DONE_HOLD: Duration = Duration::from_millis(400);
/// How long an error line stays.
const ERROR_HOLD: Duration = Duration::from_millis(2500);
/// How long "Pasted" or "Copied" shows (docs/ux/insertion.md).
const NOTICE_HOLD: Duration = Duration::from_millis(1200);
/// The exit transition (--dur-exit); the window hides once it has played.
const EXIT: Duration = Duration::from_millis(160);
/// The level feed's rate while the Bar is listening.
const LEVEL_EVERY: Duration = Duration::from_millis(16);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BarState {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    hands_free: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// For a notice: it did what was asked (a check), or not (amber).
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
}

impl BarState {
    fn new(state: &'static str) -> Self {
        Self { state, hands_free: None, message: None, ok: None }
    }
}

#[derive(Clone, Serialize)]
struct BarWords {
    committed: String,
    preview: String,
    /// The text being pasted; until then, everything shown is provisional.
    #[serde(rename = "final")]
    is_final: bool,
}

/// Shows and hides the Bar. Every dictation bumps `generation`, so a timer left over from the
/// last one (hide after Done, say) never hides the Bar of the next.
#[derive(Clone)]
struct Bar {
    window: WebviewWindow,
    /// Read once, on the UI thread: asking for it from another thread waits for the UI thread,
    /// and dictation must never wait on the UI.
    hwnd: Option<isize>,
    generation: Arc<AtomicU64>,
    listening: Arc<AtomicBool>,
    /// The microphone's level, for the waveform; replaced when the microphone changes.
    meter: Arc<Mutex<Option<utter_audio::LevelMeter>>>,
}

/// What the Home page shows about a settings change taking effect (docs/ux/home.md).
#[derive(Clone, Serialize)]
struct Applied {
    what: &'static str,
    /// "loading", "ok" or "error".
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Bar {
    fn new(window: WebviewWindow) -> Self {
        // Windows places the Bar through utter_platform::overlay, by its HWND. Elsewhere Tauri
        // does (bar_bottom_center): on Linux the window runs under XWayland, where an X11 client
        // may keep itself above others and place itself (docs/plan-cross-platform.md).
        #[cfg(windows)]
        let hwnd = window.hwnd().ok().map(|h| h.0 as isize);
        #[cfg(not(windows))]
        let hwnd: Option<isize> = None;
        if let Some(h) = hwnd {
            overlay::make_overlay(h);
        }
        #[cfg(not(windows))]
        let _ = window.set_focusable(false);
        let _ = window.set_ignore_cursor_events(true);
        Self {
            window,
            hwnd,
            generation: Arc::new(AtomicU64::new(0)),
            listening: Arc::new(AtomicBool::new(false)),
            meter: Arc::new(Mutex::new(None)),
        }
    }

    /// Shows the Bar at the bottom centre of the monitor holding `near` (a window, from focus),
    /// without activating it and without waiting for the UI thread.
    fn show(&self, near: Option<isize>) {
        if let Some(h) = self.hwnd {
            overlay::show_bottom_center(h, near, BAR_SIZE, BAR_BOTTOM_MARGIN);
            return;
        }
        let window = self.window.clone();
        let _ = self.window.run_on_main_thread(move || bar_bottom_center(&window));
    }

    fn hide(&self) {
        match self.hwnd {
            Some(h) => overlay::hide(h),
            None => {
                let _ = self.window.hide();
            }
        }
    }

    fn emit_state(&self, state: BarState) {
        let _ = self.window.emit_to(self.window.label(), "bar:state", state);
    }

    fn on_event(&self, event: Event) {
        match event {
            Event::Listening { hands_free, target } => {
                let first = !self.listening.swap(true, Ordering::Relaxed);
                if first {
                    self.generation.fetch_add(1, Ordering::Relaxed);
                }
                // State first, so the page has reset before the window becomes visible.
                self.emit_state(BarState { hands_free: Some(hands_free), ..BarState::new("listening") });
                if first {
                    self.show(target);
                }
            }
            Event::Words { committed, preview } => {
                let words = BarWords { committed, preview, is_final: false };
                let _ = self.window.emit_to(self.window.label(), "bar:words", words);
            }
            Event::Final { text } => {
                let words = BarWords { committed: text, preview: String::new(), is_final: true };
                let _ = self.window.emit_to(self.window.label(), "bar:words", words);
            }
            Event::Finishing => {
                self.listening.store(false, Ordering::Relaxed);
                self.emit_state(BarState::new("finishing"));
            }
            Event::Done(d) => {
                self.listening.store(false, Ordering::Relaxed);
                // History shows it at once (docs/ux/history.md, "Live").
                let _ = self.window.emit_to("home", "history-changed", ());
                match (&d.inserted, d.text.trim().is_empty()) {
                    (_, true) => self.leave_with_error("Didn't catch that"),
                    (Err(e), false) => {
                        tracing::warn!("not inserted: {e}");
                        self.leave_with_error(&e.message(d.copied));
                    }
                    (Ok(()), false) => {
                        self.emit_state(BarState::new("done"));
                        self.leave_after(DONE_HOLD);
                    }
                }
                tracing::info!(
                    "dictation: {:.1} s audio, {} segments, gate {:?}, asr {:.0} ms, cleanup {:.0} ms, insert {:.0} ms, total {:.0} ms",
                    d.audio_s,
                    d.segments,
                    d.gate,
                    d.asr_ms,
                    d.cleanup_ms,
                    d.insert_ms,
                    d.total_ms
                );
            }
            Event::Cancelled => {
                self.listening.store(false, Ordering::Relaxed);
                // The page fades the pill out at once; hide the window when that has played.
                self.emit_state(BarState::new("cancelled"));
                self.hide_after(EXIT);
            }
            Event::Applied { what, error } => {
                match &error {
                    Some(e) => tracing::warn!("{what} not applied: {e}"),
                    None => tracing::info!("{what} applied"),
                }
                let status = if error.is_some() { "error" } else { "ok" };
                if what == "key" && error.is_none() {
                    set_tray_status(self.window.app_handle(), &ready_status());
                }
                // A model switch setup downloaded for: setup closes once it took, or says why not.
                if what == "model" {
                    let app = self.window.app_handle();
                    let models = app.state::<home::Models>();
                    let chosen = setup::chosen_model();
                    match &error {
                        None => models.set_running(chosen),
                        // Not tried again with every later save (home::Models::for_pipeline).
                        Some(_) => models.set_failed(chosen),
                    }
                    // A model switch setup downloaded for: setup closes once it took, or says why not.
                    if app.get_webview_window("setup").is_some() {
                        match &error {
                            None => setup::ready(app),
                            Some(e) => setup::failed(app, problem::model_switch(e)),
                        }
                    }
                }
                let _ = self.window.emit_to("home", "home-applied", Applied { what, status, error });
            }
            Event::ModelLoading => {
                let _ = self.window.emit_to(
                    "home",
                    "home-applied",
                    Applied { what: "model", status: "loading", error: None },
                );
            }
            Event::Microphone(meter) => *self.meter.lock().expect("meter lock") = Some(meter),
            Event::Notice { text, ok } => {
                // Outside a dictation: a new generation, so an old timer cannot hide it.
                self.generation.fetch_add(1, Ordering::Relaxed);
                self.emit_state(BarState { message: Some(text), ok: Some(ok), ..BarState::new("notice") });
                self.show(None);
                self.leave_after(if ok { NOTICE_HOLD } else { ERROR_HOLD });
            }
        }
    }

    /// Dictation stopped with an error, possibly mid-dictation: the Bar stops listening and
    /// leaves with why, instead of staying up.
    fn stopped(&self, why: &str) {
        self.listening.store(false, Ordering::Relaxed);
        self.leave_with_error(why);
    }

    fn leave_with_error(&self, message: &str) {
        self.emit_state(BarState { message: Some(message.to_owned()), ..BarState::new("error") });
        self.leave_after(ERROR_HOLD);
    }

    /// Plays the exit after `hold`, then hides the window.
    fn leave_after(&self, hold: Duration) {
        let bar = self.clone();
        let generation = self.generation.load(Ordering::Relaxed);
        std::thread::spawn(move || {
            std::thread::sleep(hold);
            if bar.current(generation) {
                bar.emit_state(BarState::new("exit"));
                bar.hide_after(EXIT);
            }
        });
    }

    fn hide_after(&self, delay: Duration) {
        let bar = self.clone();
        let generation = self.generation.load(Ordering::Relaxed);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if bar.current(generation) {
                bar.hide();
                bar.emit_state(BarState::new("hidden"));
            }
        });
    }

    fn current(&self, generation: u64) -> bool {
        !self.listening.load(Ordering::Relaxed) && self.generation.load(Ordering::Relaxed) == generation
    }
}

/// Feeds the microphone level to the Bar while it is listening.
fn level_feed(bar: Bar) {
    // One feed for the app's life: a pipeline started again (Retry) reuses it.
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::Builder::new()
        .name("utter-levels".into())
        .spawn(move || {
            loop {
                std::thread::sleep(LEVEL_EVERY);
                if bar.listening.load(Ordering::Relaxed) {
                    let level = bar.meter.lock().expect("meter lock").as_ref().map_or(0.0, |m| m.level());
                    let _ = bar.window.emit_to(bar.window.label(), "bar:levels", [level]);
                }
            }
        })
        .expect("spawn the level feed");
}

/// What starting dictation needs, kept until first run has installed the components.
struct Core {
    bar: Bar,
    link: home::PipelineLink,
    /// Where the tray's paste and copy last go, once dictation runs.
    commands: Arc<Mutex<Option<utter_pipeline::Commands>>>,
    started: AtomicBool,
}

/// Starts dictation once (at launch, or when setup has installed everything). Returns whether
/// this call started it (false: it was running or starting already).
pub fn start_pipeline(app: &AppHandle) -> bool {
    let core = app.state::<Core>();
    let start = !core.started.swap(true, Ordering::Relaxed);
    if start {
        run_pipeline(app.clone(), core.bar.clone(), core.link.clone(), core.commands.clone());
    }
    start
}

/// Sends the settings file to the running pipeline again (after setup downloaded what they
/// need, which the watcher held them back for).
pub fn reapply_settings(app: &AppHandle) {
    match Settings::load_or_create(&Settings::path()) {
        Ok(s) => {
            if let Some(tx) = app.state::<Core>().link.0.lock().expect("pipeline link").as_ref() {
                let _ = tx.send(app.state::<home::Models>().for_pipeline(s));
            }
        }
        Err(e) => tracing::warn!("settings: {e}"),
    }
}

/// Paste or copy last from the tray; before dictation runs there is nothing to paste.
fn send_command(app: &AppHandle, c: utter_pipeline::Command) {
    match app.state::<Core>().commands.lock().expect("commands").as_ref() {
        Some(commands) => commands.send(c),
        None => tracing::info!("{c:?} before dictation started: nothing to do"),
    }
}

/// Loads the pipeline and runs dictations for as long as the app lives.
fn run_pipeline(
    app: AppHandle,
    bar: Bar,
    link: home::PipelineLink,
    commands: Arc<Mutex<Option<utter_pipeline::Commands>>>,
) {
    std::thread::Builder::new()
        .name("utter-pipeline".into())
        .spawn(move || {
            let start = std::time::Instant::now();
            tracing::info!("loading starts at {:.2} s", since_launch());
            let path = Settings::path();
            // A file with an error: dictation starts on the defaults, and Home opens on its
            // banner so the error gets fixed (docs/ux/home.md).
            let mut settings = Settings::load_or_create(&path).unwrap_or_else(|e| {
                tracing::error!("settings: {e}");
                home::show(&app);
                Settings::default()
            });
            // The chosen model still downloading: dictation starts on the one that is here.
            if let Some(model) = setup::fallback_model() {
                tracing::info!(?model, "the chosen model is not downloaded yet; starting on this one");
                settings.model = model;
            }
            tracing::info!(terms = settings.dictionary.len(), lists = settings.lists, "settings loaded");
            home::apply_autostart(&settings);
            let mut cfg = PipelineConfig::default();
            settings.apply(&mut cfg);
            utter_pipeline::apply_test_audio(&mut cfg);
            let tier = cfg.tier;
            let reporting = app.clone();
            let mut pipeline = match Pipeline::start_reporting(cfg, |s| splash::stage(&reporting, s)) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("could not start: {e}");
                    set_tray_status(&app, "Utter: could not start");
                    // Retry (first run's or the problem screen's) may start it again.
                    app.state::<Core>().started.store(false, Ordering::Relaxed);
                    // The problem screen opens first, then the card goes: no blank moment.
                    setup::failed(&app, problem::diagnose(&e, tier));
                    splash::close(&app);
                    return;
                }
            };
            tracing::info!(
                "ready at {:.2} s after launch (loading {:.2} s)",
                since_launch(),
                start.elapsed().as_secs_f64()
            );
            let models = app.state::<home::Models>();
            models.set_running(settings.model);
            // Started while something is still to download: on earlier versions (an update) or
            // the other model; the download's end switches it.
            let downloading = !setup::missing().is_empty();
            if downloading {
                models.set_on_earlier();
            }
            set_tray_status(&app, &ready_status());
            // First run's or Retry's setup window learns dictation runs. One still downloading
            // (the chosen model, an update) is not done yet: it hears from the switch.
            if !downloading {
                setup::ready(&app);
            }
            splash::stage(&app, "ready");
            *bar.meter.lock().expect("meter lock") = Some(pipeline.microphone().level_meter());
            level_feed(bar.clone());
            *link.0.lock().expect("pipeline link") = Some(pipeline.settings_sender());
            *commands.lock().expect("commands") = Some(pipeline.commands());
            if let Err(e) = pipeline.run(|event| bar.on_event(event)) {
                tracing::error!("dictation stopped: {e}");
                bar.stopped("Dictation stopped");
                set_tray_status(&app, "Utter: stopped");
                *link.0.lock().expect("pipeline link") = None;
                *commands.lock().expect("commands") = None;
                app.state::<home::Models>().stopped();
                drop(pipeline);
                app.state::<Core>().started.store(false, Ordering::Relaxed);
                setup::failed(&app, problem::diagnose(&e, tier));
            }
        })
        .expect("spawn the pipeline");
}

/// The tray's tooltip while dictation runs, naming the key to hold.
fn ready_status() -> String {
    format!("Utter: hold {} to dictate", home::dictation_key_name())
}

/// Sets the tray's tooltip on the UI thread without waiting for it: a tray call from another
/// thread blocks until the UI thread has run it, and dictation must never wait on the UI.
fn set_tray_status(app: &AppHandle, text: &str) {
    let (handle, text) = (app.clone(), text.to_owned());
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id("main") {
            let _ = tray.set_tooltip(Some(text));
        }
    });
}

/// When the process started: startup is measured from here, the wait a person sees.
static LAUNCHED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Seconds since the process started.
fn since_launch() -> f64 {
    LAUNCHED.get().map_or(0.0, |t| t.elapsed().as_secs_f64())
}

/// Places the Bar at the bottom centre of the work area of the monitor under the mouse (the
/// primary one if unknown), scaled to it, and shows it: utter_platform::overlay's job on Windows,
/// done here through Tauri elsewhere. Runs on the UI thread, which owns the window.
fn bar_bottom_center(window: &WebviewWindow) {
    let monitor = window
        .cursor_position()
        .ok()
        .and_then(|p| window.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let px = |v: u32| (v as f64 * scale).round() as i32;
        let (w, h) = (px(BAR_SIZE.0), px(BAR_SIZE.1));
        let area = m.work_area();
        let x = area.position.x + (area.size.width as i32 - w) / 2;
        let y = area.position.y + area.size.height as i32 - h - px(BAR_BOTTOM_MARGIN);
        let _ = window.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = window.show();
    // Some window managers forget "above" while a window is hidden.
    let _ = window.set_always_on_top(true);
}

/// Linux: the windows run under XWayland on a Wayland desktop, the only way there for a window
/// to stay above others and place itself (the Bar); and WebKitGTK's DMA-BUF renderer, which
/// draws blank windows on some GPUs and VMs, is off. Either is the person's to override.
#[cfg(target_os = "linux")]
fn linux_display_defaults() {
    for (key, value) in [("GDK_BACKEND", "x11,wayland"), ("WEBKIT_DISABLE_DMABUF_RENDERER", "1")] {
        if std::env::var_os(key).is_none() {
            // SAFETY: at the top of main, before any other thread exists.
            unsafe { std::env::set_var(key, value) };
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    linux_display_defaults();
    if !args::handle() {
        return;
    }
    LAUNCHED.get_or_init(std::time::Instant::now);
    tracing_subscriber_init();
    if let Some(old) = utter_core::move_legacy_data() {
        tracing::info!("moved {} to {}", old.display(), utter_core::data_dir().display());
    }
    tauri::Builder::default()
        // Launching Utter again while it runs opens the Home window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| home::show(app)))
        .manage(setup::MicCheck::default())
        .manage(setup::Downloading::default())
        .manage(setup::LastProblem::default())
        .invoke_handler(tauri::generate_handler![
            home::home_state,
            home::home_quit,
            home::dictation_key,
            home::save_settings,
            home::open_path,
            history::history_query,
            history::history_favorite,
            history::history_delete,
            history::history_restore,
            history::history_copy,
            history::history_changes,
            history::history_count_older,
            history::history_clear,
            history::paste_rules,
            history::open_paste_rules,
            setup::setup_check,
            setup::setup_allow_input,
            setup::setup_microphones,
            setup::setup_mic_start,
            setup::setup_mic_stop,
            setup::setup_download,
            setup::setup_finish,
            setup::setup_problem,
            setup::setup_retry,
            setup::setup_repair,
            setup::setup_models,
            setup::setup_choose,
            splash::splash_close,
            splash::splash_stage,
        ])
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else { return };
            match window.label() {
                // Closing Home hides it; Utter keeps running in the tray.
                "home" => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Before dictation runs nothing works without the models: closing setup quits
                // (downloads resume on the next start). Once it runs (a model change from
                // Home), closing only closes. Finishing destroys the window instead.
                "setup" if !window.app_handle().state::<Core>().started.load(Ordering::Relaxed) => {
                    window.app_handle().exit(0);
                }
                _ => {}
            }
        })
        .setup(|app| {
            tracing::info!("setup at {:.2} s", since_launch());
            watch_ui_thread(app.handle().clone());
            let open = MenuItem::with_id(app, "open", "Open Utter", true, None::<&str>)?;
            // Their shortcuts are shown beside them (docs/ux/insertion.md).
            let paste = MenuItem::with_id(app, "paste-last", "Paste last dictation", true, Some("Alt+Shift+Z"))?;
            let copy = MenuItem::with_id(app, "copy-last", "Copy last dictation", true, Some("Alt+Shift+X"))?;
            let retry = MenuItem::with_id(app, "retry-last", "Retry last dictation", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Utter", true, None::<&str>)?;
            let line = || PredefinedMenuItem::separator(app);
            let menu = Menu::with_items(app, &[&open, &line()?, &paste, &copy, &retry, &line()?, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("an app icon"))
                .tooltip("Utter: loading...")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => home::show(app),
                    "paste-last" => send_command(app, utter_pipeline::Command::PasteLast),
                    "copy-last" => send_command(app, utter_pipeline::Command::CopyLast),
                    "retry-last" => send_command(app, utter_pipeline::Command::RetryLast),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            let window = app.get_webview_window("bar").expect("the bar window (tauri.conf.json)");
            let link = home::PipelineLink::default();
            app.manage(home::Models::default());
            home::watch(app.handle().clone(), link.clone());
            app.manage(Core { bar: Bar::new(window), link, commands: Arc::default(), started: AtomicBool::new(false) });
            // A repair that found its components in use restarted Utter to remove them now,
            // before anything loads them.
            let repairing = setup::finish_pending_repair();
            // What an update left behind goes first: it may be the disk space the next
            // download needs.
            let freed = utter_runtime::prune();
            if freed > 0 {
                tracing::info!("removed superseded components: {:.1} GB", freed as f64 / 1e9);
            }
            let missing = setup::missing();
            if !missing.is_empty() {
                // A download is never started without saying what is missing and where it looked.
                for c in &missing {
                    tracing::warn!(
                        id = %c.id,
                        version = %c.version,
                        dir = %c.dir().display(),
                        installed = c.is_installed(),
                        "setup needed: not installed, or a file is missing or cut short"
                    );
                }
            }
            match setup::mode_for(&missing) {
                None => {
                    setup::mark_setup_complete();
                    // Loading first: building the card's window takes a moment it need not wait.
                    start_pipeline(app.handle());
                    // The person started Utter: the card shows while it loads. Not at sign-in.
                    if !splash::started_at_sign_in() {
                        splash::open(app.handle());
                    }
                }
                // The chosen model is not here yet but the other one is: dictation starts on that
                // (run_pipeline), and the chosen one downloads beside it.
                Some(_) if !repairing && setup::fallback_model().is_some() => {
                    start_pipeline(app.handle());
                    setup::open(app.handle(), setup::Mode::Model)?;
                }
                // An update: dictation runs on the versions here (utter_runtime keeps them until
                // the new ones are whole), and switches when the download is done.
                Some(setup::Mode::Update) if !repairing => {
                    start_pipeline(app.handle());
                    setup::open(app.handle(), setup::Mode::Update)?;
                }
                Some(mode) => {
                    let mode = if repairing && mode == setup::Mode::Update { setup::Mode::Repair } else { mode };
                    set_tray_status(app.handle(), "Utter: setting up");
                    setup::open_with(app.handle(), mode, repairing)?;
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run Utter");
}

/// Logs to stderr (a development build's console) and to the rotating `logs/utter.log`
/// (utter_core::logs), which settings.toml `keep_logs` turns off. Read before anything logs, so
/// a person who turned logs off never gets one.
fn tracing_subscriber_init() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{EnvFilter, fmt};
    let keep = Settings::load_or_create(&Settings::path()).map_or(true, |s| s.keep_logs);
    utter_core::logs::set_enabled(keep);
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ort=warn"));
    let file = Mutex::new(utter_core::logs::RotatingFile::new("utter.log"));
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_filter(filter()))
        .with(fmt::layer().with_ansi(false).with_writer(file).with_filter(filter()))
        .init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Utter starts");
}

/// Logs when the UI thread is slow to answer. Nothing in dictation waits on it, but a stuck UI
/// thread delays the Bar and the windows, and this says when and for how long.
fn watch_ui_thread(app: AppHandle) {
    /// When the pending check was sent (ms since launch), 0 when none is pending.
    static PENDING: AtomicU64 = AtomicU64::new(0);
    let ms = || (since_launch() * 1000.0) as u64 + 1;
    let spawned = std::thread::Builder::new().name("utter-ui-watch".into()).spawn(move || {
        let mut warned = 0;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let sent = PENDING.load(Ordering::Relaxed);
            if sent != 0 {
                let late = (ms() - sent) / 1000;
                if late >= 2 && late >= warned + 5 {
                    warned = late;
                    tracing::warn!("the UI thread has not answered for {late} s");
                }
                continue;
            }
            warned = 0;
            let now = ms();
            PENDING.store(now, Ordering::Relaxed);
            let sent = app.run_on_main_thread(move || {
                let late = ms() - now;
                PENDING.store(0, Ordering::Relaxed);
                if late > 250 {
                    tracing::warn!("the UI thread answered after {late} ms");
                }
            });
            if sent.is_err() {
                return;
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("UI watch: {e}");
    }
}
