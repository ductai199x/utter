//! Utter's app: the Bar overlay and the tray, driven by the dictation pipeline.
//!
//! The Bar window is created hidden with its page already loaded, so showing it costs one
//! frame. It never takes focus and lets every click through (docs/ux/bar.md).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use utter_pipeline::{Event, Pipeline, PipelineConfig, Settings};
use utter_platform::overlay;

mod home;
mod setup;

/// The Bar window's size in logical pixels (tauri.conf.json): room for the largest pill plus
/// its shadow.
const BAR_SIZE: (u32, u32) = (576, 168);
/// The pill sits 12 px above the window's bottom edge, and 48 px above the work area's.
const BAR_BOTTOM_MARGIN: u32 = 48 - 12;
/// How long the Done check shows before the Bar leaves.
const DONE_HOLD: Duration = Duration::from_millis(400);
/// How long an error line stays.
const ERROR_HOLD: Duration = Duration::from_millis(2500);
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
}

impl BarState {
    fn new(state: &'static str) -> Self {
        Self { state, hands_free: None, message: None }
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
        if let Ok(hwnd) = window.hwnd() {
            overlay::make_overlay(hwnd.0 as isize);
        }
        let _ = window.set_ignore_cursor_events(true);
        Self {
            window,
            generation: Arc::new(AtomicU64::new(0)),
            listening: Arc::new(AtomicBool::new(false)),
            meter: Arc::new(Mutex::new(None)),
        }
    }

    fn handle(&self) -> Option<isize> {
        self.window.hwnd().ok().map(|h| h.0 as isize)
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
                if first && let Some(h) = self.handle() {
                    overlay::show_bottom_center(h, target, BAR_SIZE, BAR_BOTTOM_MARGIN);
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
                match (&d.inserted, d.text.trim().is_empty()) {
                    (_, true) => self.leave_with_error("Didn't catch that"),
                    (Err(e), false) => {
                        tracing::warn!("insert failed: {e}");
                        self.leave_with_error(&insert_message(e));
                    }
                    (Ok(()), false) => {
                        self.emit_state(BarState::new("done"));
                        self.leave_after(DONE_HOLD);
                    }
                }
                tracing::info!(
                    "dictation: {:.1} s audio, {} segments, gate {:?}, asr {:.0} ms, cleanup {:.0} ms, total {:.0} ms",
                    d.audio_s,
                    d.segments,
                    d.gate,
                    d.asr_ms,
                    d.cleanup_ms,
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
                if let Some(e) = &error {
                    tracing::warn!("{what} not applied: {e}");
                }
                let status = if error.is_some() { "error" } else { "ok" };
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
        }
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
                if let Some(h) = bar.handle() {
                    overlay::hide(h);
                }
                bar.emit_state(BarState::new("hidden"));
            }
        });
    }

    fn current(&self, generation: u64) -> bool {
        !self.listening.load(Ordering::Relaxed) && self.generation.load(Ordering::Relaxed) == generation
    }
}

/// What the Bar says when the paste didn't happen. The text is never lost: it stays on the
/// paste-last shortcut (and in History, once there is one).
fn insert_message(error: &str) -> String {
    if error.contains("focus") {
        "Focus moved: press Shift+Alt+Z to paste"
    } else {
        "Couldn't paste: press Shift+Alt+Z"
    }
    .to_owned()
}

/// Feeds the microphone level to the Bar while it is listening.
fn level_feed(bar: Bar) {
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
    started: AtomicBool,
}

/// Starts dictation once (at launch, or when first run has installed everything).
pub fn start_pipeline(app: &AppHandle) {
    let core = app.state::<Core>();
    if !core.started.swap(true, Ordering::Relaxed) {
        run_pipeline(app.clone(), core.bar.clone(), core.link.clone());
    }
}

/// Loads the pipeline and runs dictations for as long as the app lives.
fn run_pipeline(app: AppHandle, bar: Bar, link: home::PipelineLink) {
    std::thread::Builder::new()
        .name("utter-pipeline".into())
        .spawn(move || {
            let start = std::time::Instant::now();
            let path = Settings::path();
            // A file with an error: dictation starts on the defaults, and Home opens on its
            // banner so the error gets fixed (docs/ux/home.md).
            let settings = Settings::load_or_create(&path).unwrap_or_else(|e| {
                tracing::error!("settings: {e}");
                home::show(&app);
                Settings::default()
            });
            tracing::info!(terms = settings.dictionary.len(), lists = settings.lists, "settings loaded");
            home::apply_autostart(&settings);
            let mut cfg = PipelineConfig::default();
            settings.apply(&mut cfg);
            let mut pipeline = match Pipeline::start(cfg) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("could not start: {e}");
                    set_tray_status(&app, &format!("Utter: could not start ({e})"));
                    return;
                }
            };
            tracing::info!("ready in {:.1} s", start.elapsed().as_secs_f64());
            set_tray_status(&app, "Utter: hold Right Ctrl to dictate");
            setup::ready(&app);
            *bar.meter.lock().expect("meter lock") = Some(pipeline.microphone().level_meter());
            level_feed(bar.clone());
            *link.0.lock().expect("pipeline link") = Some(pipeline.settings_sender());
            if let Err(e) = pipeline.run(|event| bar.on_event(event)) {
                tracing::error!("dictation stopped: {e}");
                set_tray_status(&app, &format!("Utter: stopped ({e})"));
            }
        })
        .expect("spawn the pipeline");
}

fn set_tray_status(app: &AppHandle, text: &str) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text));
    }
}

fn main() {
    tracing_subscriber_init();
    tauri::Builder::default()
        // Launching Utter again while it runs opens the Home window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| home::show(app)))
        .manage(setup::MicCheck::default())
        .manage(setup::Downloading::default())
        .invoke_handler(tauri::generate_handler![
            home::home_state,
            home::save_settings,
            home::open_path,
            setup::setup_check,
            setup::setup_microphones,
            setup::setup_mic_start,
            setup::setup_mic_stop,
            setup::setup_download,
            setup::setup_finish,
        ])
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else { return };
            match window.label() {
                // Closing Home hides it; Utter keeps running in the tray.
                "home" => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Nothing works without the models: closing first run quits (downloads resume
                // on the next start). Finishing it destroys the window instead.
                "setup" => window.app_handle().exit(0),
                _ => {}
            }
        })
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open Utter", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Utter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("an app icon"))
                .tooltip("Utter: loading...")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => home::show(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            let window = app.get_webview_window("bar").expect("the bar window (tauri.conf.json)");
            let link = home::PipelineLink::default();
            home::watch(app.handle().clone(), link.clone());
            app.manage(Core { bar: Bar::new(window), link, started: AtomicBool::new(false) });
            if setup::missing().is_empty() {
                start_pipeline(app.handle());
            } else {
                set_tray_status(app.handle(), "Utter: setting up");
                tauri::WebviewWindowBuilder::new(app, "setup", tauri::WebviewUrl::App("setup.html".into()))
                    .title("Utter")
                    .inner_size(640.0, 520.0)
                    .resizable(false)
                    .center()
                    .build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run Utter");
}

fn tracing_subscriber_init() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ort=warn")))
        .init();
}
