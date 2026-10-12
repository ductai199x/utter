//! The splash (docs/ux/splash.md): a frameless card while dictation loads after the person
//! starts Utter. It shows the loading stages this module reports, and fades itself out once
//! told dictation is ready. Never at sign-in.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// The flag the sign-in entry starts Utter with (utter_platform::autostart): no splash then.
pub const AUTOSTART_ARG: &str = "--autostart";

/// Whether Utter was started at sign-in rather than by the person.
pub fn started_at_sign_in() -> bool {
    std::env::args().any(|a| a == AUTOSTART_ARG)
}

/// Opens the card, just after loading starts (the page asks where loading is: splash_stage).
pub fn open(app: &AppHandle) {
    let built = tauri::WebviewWindowBuilder::new(app, "splash", tauri::WebviewUrl::App("splash.html".into()))
        .title("Utter")
        // The card is 640 x 400; the window has 20 px around it for the card's own shadow.
        .inner_size(680.0, 440.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .center()
        .build();
    if let Err(e) = built {
        tracing::warn!("splash: {e}");
    }
}

#[derive(Clone, Serialize)]
struct Stage {
    stage: &'static str,
}

/// The stage last reported, for a card that opens after loading started ([`splash_stage`]).
static LAST: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// Tells the card which stage is loading: "speech", "cleanup", "microphone", or "ready".
pub fn stage(app: &AppHandle, stage: &'static str) {
    *LAST.lock().expect("splash stage lock") = Some(stage);
    let _ = app.emit_to("splash", "splash-stage", Stage { stage });
}

/// Where loading is now: the card asks once it listens, since loading starts first.
#[tauri::command]
pub fn splash_stage() -> Option<&'static str> {
    *LAST.lock().expect("splash stage lock")
}

/// Closes the card at once (dictation could not start: the problem screen takes its place).
pub fn close(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("splash") {
        let _ = w.destroy();
    }
}

/// The card closes itself once its fade has played, or when the person dismisses it.
#[tauri::command]
pub fn splash_close(app: AppHandle) {
    close(&app);
}
