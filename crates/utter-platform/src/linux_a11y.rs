//! The focused app and the text before the caret on Linux, under X11 and Wayland alike, from
//! the accessibility bus (AT-SPI): Wayland tells a normal app nothing about other windows, and
//! AT-SPI is what screen readers use (docs/platforms.md, spike 3: on GNOME 46 Wayland
//! it named the active app and its pid, and read the text before the caret exactly).
//!
//! A thread listens for focus changes and remembers the focused object; at start one walk of the
//! tree finds what already has the focus. GTK, Qt and GNOME apps answer; Chromium and Electron
//! apps only when accessibility is turned on for them, and then this knows nothing (the caller
//! still pastes, it only cannot check the window or space the text).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// AT-SPI state bits (atspi-constants.h).
const STATE_ACTIVE: u32 = 1;
const STATE_FOCUSED: u32 = 12;
/// The most objects one search of the tree visits.
const WALK_LIMIT: usize = 4000;

#[derive(Debug, Clone)]
pub struct Focused {
    /// The app's unique name on the accessibility bus, and the focused object's path.
    pub bus: String,
    pub path: OwnedObjectPath,
    pub pid: u32,
}

static FOCUS: Mutex<Option<Focused>> = Mutex::new(None);

fn bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        let connect = || -> zbus::Result<Connection> {
            let session = Connection::session()?;
            let address: String = session
                .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())?
                .body()
                .deserialize()?;
            zbus::blocking::connection::Builder::address(address.as_str())?.build()
        };
        connect().inspect_err(|e| tracing::warn!("accessibility bus: {e}")).ok()
    })
    .as_ref()
}

/// Whether Utter turned the session's accessibility on (so turns it off at quit).
static ENABLED_BY_US: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The session's accessibility switch (org.a11y.Status IsEnabled). Qt apps (Konsole, every KDE
/// app) and Chromium join the accessibility bus only while it is on: off, as Plasma leaves it,
/// the focused terminal was unknown, so it got Ctrl+V, not its Ctrl+Shift+V, and nothing was
/// pasted (Plasma 6, 2026-10-18). Apps already running join as soon as it turns on.
fn set_accessibility(on: bool) -> zbus::Result<()> {
    let session = Connection::session()?;
    session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(on)),
    )?;
    Ok(())
}

fn accessibility_on() -> zbus::Result<bool> {
    let session = Connection::session()?;
    let value: OwnedValue = session
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.a11y.Status", "IsEnabled"),
        )?
        .body()
        .deserialize()?;
    Ok(bool::try_from(value).unwrap_or(false))
}

/// Turns the session's accessibility back off if Utter turned it on (at quit).
pub fn stop() {
    if ENABLED_BY_US.swap(false, std::sync::atomic::Ordering::Relaxed)
        && let Err(e) = set_accessibility(false)
    {
        tracing::warn!("accessibility off: {e}");
    }
}

/// Starts following the focus (once per process).
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new().name("utter-a11y".into()).spawn(|| {
            match accessibility_on() {
                Ok(false) => match set_accessibility(true) {
                    Ok(()) => {
                        ENABLED_BY_US.store(true, std::sync::atomic::Ordering::Relaxed);
                        tracing::info!("accessibility turned on, for the focused app and the caret");
                    }
                    Err(e) => tracing::warn!("accessibility on: {e}"),
                },
                Ok(true) => {}
                Err(e) => tracing::warn!("accessibility status: {e}"),
            }
            let Some(conn) = bus() else { return };
            if let Some(found) = walk_for_focus(conn) {
                *FOCUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(found);
            }
            if let Err(e) = follow(conn) {
                tracing::warn!("accessibility events: {e}");
            }
        });
    });
}

/// Keeps FOCUS up to date from "object:state-changed:focused" events.
fn follow(conn: &Connection) -> zbus::Result<()> {
    // Apps send events only for kinds someone registered for.
    conn.call_method(
        Some("org.a11y.atspi.Registry"),
        "/org/a11y/atspi/registry",
        Some("org.a11y.atspi.Registry"),
        "RegisterEvent",
        &("object:state-changed:focused",),
    )?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.a11y.atspi.Event.Object")?
        .member("StateChanged")?
        .build();
    for msg in MessageIterator::for_match_rule(rule, conn, Some(256))? {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let (Some(sender), Some(path)) = (header.sender(), header.path()) else { continue };
        let Ok((kind, gained, _, _, _)) =
            msg.body().deserialize::<(String, i32, i32, OwnedValue, HashMap<String, OwnedValue>)>()
        else {
            continue;
        };
        if kind != "focused" || gained != 1 {
            continue;
        }
        let bus = sender.to_string();
        let Some(pid) = pid_of(conn, &bus).map(app_pid) else { continue };
        *FOCUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Focused { bus, path: path.to_owned().into(), pid });
    }
    Ok(())
}

fn pid_of(conn: &Connection, bus: &str) -> Option<u32> {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetConnectionUnixProcessID",
        &(bus,),
    )
    .ok()?
    .body()
    .deserialize()
    .ok()
}

fn children(conn: &Connection, bus: &str, path: &str) -> Vec<(String, OwnedObjectPath)> {
    conn.call_method(Some(bus), path, Some("org.a11y.atspi.Accessible"), "GetChildren", &())
        .ok()
        .and_then(|m| m.body().deserialize().ok())
        .unwrap_or_default()
}

fn has_state(conn: &Connection, bus: &str, path: &str, state: u32) -> bool {
    let states: Vec<u32> = conn
        .call_method(Some(bus), path, Some("org.a11y.atspi.Accessible"), "GetState", &())
        .ok()
        .and_then(|m| m.body().deserialize().ok())
        .unwrap_or_default();
    states.get((state / 32) as usize).is_some_and(|w| w & (1 << (state % 32)) != 0)
}

/// What has the focus now, found by walking the apps' active windows.
fn walk_for_focus(conn: &Connection) -> Option<Focused> {
    let mut budget = WALK_LIMIT;
    for (app_bus, app_path) in children(conn, "org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root") {
        for (bus, window) in children(conn, &app_bus, app_path.as_str()) {
            if !has_state(conn, &bus, window.as_str(), STATE_ACTIVE) {
                continue;
            }
            let mut stack = vec![window];
            while let Some(path) = stack.pop() {
                if budget == 0 {
                    return None;
                }
                budget -= 1;
                if has_state(conn, &bus, path.as_str(), STATE_FOCUSED) {
                    let pid = app_pid(pid_of(conn, &bus)?);
                    return Some(Focused { bus, path, pid });
                }
                stack.extend(children(conn, &bus, path.as_str()).into_iter().map(|(_, p)| p));
            }
        }
    }
    None
}

/// The focused object, as last reported.
pub fn focused() -> Option<Focused> {
    start();
    FOCUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Up to `chars` characters before the caret in the focused object, if it has text.
pub fn before_caret(chars: i32) -> Option<String> {
    let f = focused()?;
    let conn = bus()?;
    // On the bus the caret is a property, CaretOffset (libatspi's get_caret_offset reads it).
    let caret: OwnedValue = conn
        .call_method(
            Some(f.bus.as_str()),
            f.path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.a11y.atspi.Text", "CaretOffset"),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let caret = i32::try_from(caret).ok()?;
    if caret < 0 {
        return None;
    }
    conn.call_method(
        Some(f.bus.as_str()),
        f.path.as_str(),
        Some("org.a11y.atspi.Text"),
        "GetText",
        &((caret - chars).max(0), caret),
    )
    .ok()?
    .body()
    .deserialize()
    .ok()
}

/// The apps on the accessibility bus, as process ids.
pub fn app_pids() -> Vec<u32> {
    let Some(conn) = bus() else { return Vec::new() };
    let mut pids: Vec<u32> = children(conn, "org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root")
        .into_iter()
        .filter_map(|(b, _)| pid_of(conn, &b).map(app_pid))
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// The app a process belongs to: WebKitGTK draws pages in helper processes (WebKitWebProcess),
/// and the accessibility bus names the helper, so a dictation into Utter's own windows was
/// filed under "WebKitWebProcess"; its parent is the app (Utter, GNOME Web, ...).
fn app_pid(pid: u32) -> u32 {
    let helper = std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim().starts_with("WebKit"));
    if !helper {
        return pid;
    }
    // /proc/<pid>/stat: "pid (comm) state ppid ..."; comm may hold spaces, so after the last ')'.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)?.parse().ok()))
        .filter(|&ppid: &u32| ppid > 1)
        .unwrap_or(pid)
}

/// A process's program name and its executable's path. The name is the executable's file name;
/// `/proc/<pid>/comm` is cut at 15 characters ("gnome-terminal-" for gnome-terminal-server),
/// so it is only the fallback when the executable cannot be read.
pub fn process(pid: u32) -> Option<(String, String)> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.display().to_string()).unwrap_or_default();
    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .or_else(|| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|c| c.trim().to_owned()))?;
    Some((name, path))
}
