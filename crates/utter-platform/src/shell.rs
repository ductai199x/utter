//! The desktop shell: opening a file in the app the user has chosen for its type (settings.toml
//! in their editor), and whether it shows tray icons.

use std::path::Path;
use std::process::Command;

/// Opens `path` with its default app and returns without waiting for it.
pub fn open(path: &Path) -> std::io::Result<()> {
    imp::command(path).spawn().map(drop)
}

/// Whether the desktop shows tray icons. Windows and macOS always do; on Linux, a desktop
/// without a StatusNotifier host (GNOME without an AppIndicator extension, as Fedora ships it)
/// shows none, and the app's tray menu (Quit among it) must be offered elsewhere.
pub fn tray_shown() -> bool {
    #[cfg(target_os = "linux")]
    {
        let has_watcher = || -> zbus::Result<bool> {
            let conn = zbus::blocking::Connection::session()?;
            conn.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &("org.kde.StatusNotifierWatcher",),
            )?
            .body()
            .deserialize()
        };
        has_watcher().unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    true
}

#[cfg(windows)]
mod imp {
    use super::*;

    // Explorer hands the file to its associated app (and exits 1 even then: only failing to
    // start counts).
    pub fn command(path: &Path) -> Command {
        let mut cmd = Command::new("explorer.exe");
        cmd.arg(path);
        cmd
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn command(path: &Path) -> Command {
        let mut cmd = Command::new("open");
        cmd.arg(path);
        cmd
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    pub fn command(path: &Path) -> Command {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(path);
        cmd
    }
}
