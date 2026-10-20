//! New versions of Utter itself (the models update through setup, utter_runtime). While the
//! setting is on, Utter asks GitHub once a day whether a newer release is out: one request for a
//! public file (the release's latest.json), nothing about the person in it. A newer version is
//! only offered, in Settings and the tray menu; nothing is downloaded or installed until the
//! person clicks Update. The download is checked against the updater key built into the app
//! (tauri.conf.json, plugins.updater.pubkey) before it runs.
//!
//! Each install takes its own kind of file (packaging/latest-json.sh names them per installer):
//! Windows runs the new installer, an AppImage writes the new one over itself, a .deb installs
//! the new .deb through the system's password prompt (pkexec), and macOS replaces Utter.app,
//! signed with the same certificate, so its permissions stay.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The first check, after Utter is ready and has settled.
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// How often the check runs while the setting is on.
const EVERY: Duration = Duration::from_secs(24 * 3600);
/// How often the background thread looks whether a check is due (and at the setting).
const LOOK: Duration = Duration::from_secs(600);

/// Where an update stands, for Settings and the tray.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum Status {
    #[default]
    Idle,
    Checking,
    /// This is the newest version.
    Current {
        version: String,
    },
    /// `version` is out; `password`: installing it asks for the system password (a .deb).
    #[serde(rename_all = "camelCase")]
    Available {
        version: String,
        notes: Option<String>,
        password: bool,
    },
    /// Bytes downloaded, of `total` when the server says.
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    /// Downloaded and checked; installing, then Utter starts again.
    Installing,
    Failed {
        error: String,
    },
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    pending: Mutex<Option<Update>>,
    checked: Mutex<Option<Instant>>,
}

/// Whether this copy was installed from the .deb (Linux, not the AppImage): its update asks
/// for the system password.
fn from_deb() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none_or(|p| p.is_empty())
}

fn set(app: &AppHandle, status: Status) {
    *app.state::<Updates>().status.lock().expect("update status lock") = status.clone();
    let _ = app.emit_to("home", "update-status", &status);
    crate::set_tray_update(app, &status);
}

pub fn status(app: &AppHandle) -> Status {
    app.state::<Updates>().status.lock().expect("update status lock").clone()
}

/// Checks in the background: first a minute after Utter is ready, then once a day, while the
/// setting is on. A development build checks only when UTTER_UPDATE_URL names a test server.
pub fn start(app: &AppHandle) {
    if utter_core::is_development_build() && std::env::var_os("UTTER_UPDATE_URL").is_none() {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new().name("utter-update".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            let on = utter_pipeline::settings::Settings::load_or_create(&utter_pipeline::settings::Settings::path())
                .map_or(true, |s| s.check_updates);
            let due =
                app.state::<Updates>().checked.lock().expect("update check lock").is_none_or(|t| t.elapsed() >= EVERY);
            let busy =
                matches!(status(&app), Status::Downloading { .. } | Status::Installing | Status::Available { .. });
            if on && due && !busy {
                tauri::async_runtime::block_on(check(&app, false));
            }
            std::thread::sleep(LOOK);
        }
    });
}

/// Asks whether a newer version is out. `by_hand`: the person clicked Check now, so the
/// result shows even when there is nothing new or the check failed.
pub async fn check(app: &AppHandle, by_hand: bool) {
    if by_hand {
        set(app, Status::Checking);
    }
    *app.state::<Updates>().checked.lock().expect("update check lock") = Some(Instant::now());
    let mut builder = app.updater_builder();
    // A test server (a release candidate's latest.json); the signature is checked all the same.
    if let Some(url) = std::env::var("UTTER_UPDATE_URL").ok().and_then(|u| tauri::Url::parse(&u).ok()) {
        builder = match builder.endpoints(vec![url]) {
            Ok(b) => b,
            Err(e) => return failed(app, by_hand, e.to_string()),
        };
    }
    let found = match builder.build() {
        Ok(updater) => updater.check().await,
        Err(e) => return failed(app, by_hand, e.to_string()),
    };
    match found {
        Ok(Some(update)) => {
            tracing::info!(version = %update.version, "a new version of Utter is out");
            let next =
                Status::Available { version: update.version.clone(), notes: update.body.clone(), password: from_deb() };
            *app.state::<Updates>().pending.lock().expect("update lock") = Some(update);
            set(app, next);
        }
        Ok(None) => {
            tracing::info!("Utter is up to date");
            set(app, Status::Current { version: app.package_info().version.to_string() });
        }
        Err(e) => failed(app, by_hand, e.to_string()),
    }
}

/// A failed check says so when asked by hand; a background one stays quiet (offline, a
/// firewall) and tries again tomorrow.
fn failed(app: &AppHandle, by_hand: bool, error: String) {
    tracing::warn!("update check: {error}");
    if by_hand {
        set(app, Status::Failed { error: "Could not reach GitHub to check. Try again later.".into() });
    }
}

/// The person clicked Update: downloads, checks the signature, installs and starts the new
/// version. On Windows the installer closes Utter and opens the new one itself.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let Some(update) = app.state::<Updates>().pending.lock().expect("update lock").clone() else {
        return Err("no update to install".into());
    };
    set(app, Status::Downloading { done: 0, total: None });
    let mut done = 0u64;
    let mut shown = Instant::now() - Duration::from_secs(1);
    let progress_app = app.clone();
    // The download is checked against the updater key before it is returned.
    let downloaded = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                // A few times a second is enough for a progress line.
                if shown.elapsed() >= Duration::from_millis(250) {
                    shown = Instant::now();
                    set(&progress_app, Status::Downloading { done, total });
                }
            },
            || {},
        )
        .await;
    let bytes = match downloaded {
        Ok(bytes) => bytes,
        Err(e) => return fail(app, &e.to_string()),
    };
    set(app, Status::Installing);
    let installed = if from_deb() {
        install_deb(&update.version, &bytes)
    } else {
        update.install(&bytes).map_err(|e| e.to_string())
    };
    match installed {
        Ok(()) => {
            tracing::info!(version = %update.version, "update installed: starting the new version");
            crate::restart(app);
            Ok(())
        }
        Err(e) if e == DECLINED => {
            // The person said no to the password prompt: offered again, nothing changed.
            tracing::info!("update: the password prompt was declined");
            set(app, Status::Available { version: update.version.clone(), notes: update.body.clone(), password: true });
            Ok(())
        }
        Err(e) => fail(app, &e),
    }
}

fn fail(app: &AppHandle, error: &str) -> Result<(), String> {
    tracing::error!("update: {error}");
    set(app, Status::Failed { error: format!("The update did not install: {error}") });
    Err(error.to_owned())
}

/// What install_deb returns when the person cancels the password prompt.
const DECLINED: &str = "declined";

/// Installs a downloaded .deb through the desktop's own password prompt (pkexec). The plugin's
/// own install went on to a second prompt (zenity, then kdialog, then sudo) when the first was
/// cancelled (Ubuntu 24.04, 2026-10-20): one "no" is the answer.
fn install_deb(version: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("utter-update-{version}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let deb = dir.join(format!("Utter_{version}_amd64.deb"));
    std::fs::write(&deb, bytes).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("pkexec").arg("dpkg").arg("-i").arg(&deb).status();
    let _ = std::fs::remove_dir_all(&dir);
    match status.map(|s| s.code()) {
        Ok(Some(0)) => Ok(()),
        // pkexec: 126, the prompt dismissed or the password refused; 127, not authorized.
        Ok(Some(126 | 127)) => Err(DECLINED.to_owned()),
        Ok(code) => Err(format!("dpkg ended with {code:?}")),
        Err(e) => Err(format!("could not run pkexec: {e}")),
    }
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> Status {
    status(&app)
}

#[tauri::command]
pub async fn update_check(app: AppHandle) {
    check(&app, true).await
}

#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    install(&app).await
}
