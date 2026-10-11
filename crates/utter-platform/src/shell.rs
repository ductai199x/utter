//! Opening a file in the app the user has chosen for its type (settings.toml in their editor).

use std::path::Path;
use std::process::Command;

/// Opens `path` with its default app and returns without waiting for it.
pub fn open(path: &Path) -> std::io::Result<()> {
    imp::command(path).spawn().map(drop)
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
