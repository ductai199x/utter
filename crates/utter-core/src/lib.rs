//! Shared pieces of Utter's core: where data lives, and tracing setup.

use std::path::PathBuf;

/// Utter's data folder: `UTTER_DATA_DIR` if set, else `~/.cache/utter` (Tai, 2026-10-09).
/// The Python tools resolve the same folder (`tools/eval/paths.py`); `docs/local-setup.md`
/// describes its layout.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("UTTER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    home_dir().join(".cache").join("utter")
}

/// Where exported models live: `<data_dir>/models/<name>`.
pub fn model_dir(name: &str) -> PathBuf {
    data_dir().join("models").join(name)
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Installs a tracing subscriber that writes to stderr, filtered by `RUST_LOG` (default:
/// `info` for Utter's crates, `warn` for everything else).
pub fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,utter_core=info,utter_asr=info,utter_cli=info"));
    let _ = fmt().with_env_filter(filter).with_writer(std::io::stderr).try_init();
}
