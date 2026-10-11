//! Shared pieces of Utter's core: where data lives, and tracing setup.

use std::path::{Path, PathBuf};

/// Utter's data folder: `UTTER_DATA_DIR` if set, else the platform's place for per-user app
/// data, in a folder of its own for an installed copy and a sibling for development (Tai,
/// 2026-10-11):
///
/// | | Windows | Linux | macOS |
/// |---|---|---|---|
/// | installed | `%LOCALAPPDATA%\Utter` | `~/.local/share/utter` | `~/Library/Application Support/Utter` |
/// | development | `%LOCALAPPDATA%\Utter_dev` | `~/.local/share/utter_dev` | `.../Utter_dev` |
///
/// Development means a build running from a cargo `target` folder; the Python tools resolve
/// the development folder too (`tools/eval/paths.py`). The two never share state, so a model
/// or setting under development cannot reach the installed app. `docs/local-setup.md`
/// describes the layout.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("UTTER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    platform_dir().join(folder_name(is_development_build()))
}

/// Whether this program runs from a cargo `target` folder (a development build).
pub fn is_development_build() -> bool {
    std::env::current_exe().is_ok_and(|exe| exe.components().any(|c| c.as_os_str() == "target"))
}

fn folder_name(development: bool) -> &'static str {
    match (cfg!(target_os = "linux"), development) {
        (true, false) => "utter",
        (true, true) => "utter_dev",
        (false, false) => "Utter",
        (false, true) => "Utter_dev",
    }
}

/// The platform's folder for per-user app data that stays on this machine.
fn platform_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(dir);
        }
        return home_dir().join("AppData").join("Local");
    }
    if cfg!(target_os = "macos") {
        return home_dir().join("Library").join("Application Support");
    }
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home_dir().join(".local").join("share"),
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Moves the folder Utter used before 2026-10-11 (`~/.cache/utter`) to the development data
/// folder, once: that folder held development work (eval sets, training runs, the models the
/// dev builds installed). A rename on the same drive, so nothing is copied. Returns the old
/// path when it moved.
pub fn move_legacy_data() -> Option<PathBuf> {
    if std::env::var_os("UTTER_DATA_DIR").is_some() {
        return None;
    }
    let legacy = home_dir().join(".cache").join("utter");
    let target = platform_dir().join(folder_name(true));
    move_if_new(&legacy, &target).then_some(legacy)
}

fn move_if_new(from: &Path, to: &Path) -> bool {
    if !from.is_dir() || to.exists() {
        return false;
    }
    if let Some(parent) = to.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(from, to) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("could not move {} to {}: {e}", from.display(), to.display());
            false
        }
    }
}

/// Installs a tracing subscriber that writes to stderr, filtered by `RUST_LOG` (default:
/// `info` for Utter's crates, `warn` for everything else).
pub fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,utter_core=info,utter_asr=info,utter_cli=info"));
    let _ = fmt().with_env_filter(filter).with_writer(std::io::stderr).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_and_development_folders_are_siblings() {
        let (installed, development) = (folder_name(false), folder_name(true));
        assert_ne!(installed, development);
        assert!(development.starts_with(installed) && development.ends_with("_dev"));
    }

    #[test]
    fn a_legacy_folder_moves_once_and_never_over_an_existing_one() {
        let root = std::env::temp_dir().join(format!("utter-move-{}", std::process::id()));
        let (from, to) = (root.join("old"), root.join("new"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("settings.toml"), "lists = true\n").unwrap();
        assert!(move_if_new(&from, &to));
        assert!(to.join("settings.toml").is_file() && !from.exists());
        std::fs::create_dir_all(&from).unwrap();
        assert!(!move_if_new(&from, &to), "an existing folder is never replaced");
        let _ = std::fs::remove_dir_all(&root);
    }
}
