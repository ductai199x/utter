//! Utter's log files, in `<data_dir>/logs`: `utter.log` (the app's tracing) and
//! `llama-server.log` (the cleanup server's output). Each is capped at [`MAX_BYTES`]; when it
//! fills it becomes `<name>.1.log`, the older ones move up, and [`KEEP`] old files stay. They
//! hold timings, stages and errors, never what was said.
//!
//! The person can turn logging off (settings.toml `keep_logs`): then nothing is written and the
//! old files are deleted. [`set_enabled`] applies that to every writer in this process.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// A log file's size before it is rotated.
pub const MAX_BYTES: u64 = 5_000_000;
/// Rotated files kept beside the current one.
pub const KEEP: u32 = 3;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// The folder the logs are in.
pub fn dir() -> PathBuf {
    crate::data_dir().join("logs")
}

/// Whether logs are written (settings.toml `keep_logs`).
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Turns logging on or off for this process. Off deletes every log file; writers already open
/// drop their file at their next write.
pub fn set_enabled(on: bool) {
    let was = ENABLED.swap(on, Ordering::Relaxed);
    if !on && was {
        remove_all();
    }
}

/// Deletes every log file (a file in use stays until its writer lets go of it).
pub fn remove_all() {
    let Ok(entries) = fs::read_dir(dir()) else { return };
    for e in entries.flatten() {
        if e.path().extension().is_some_and(|x| x == "log") {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// `<stem>.<n>.log` beside `path` (`utter.log` -> `utter.2.log`).
fn numbered(path: &Path, n: u32) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{stem}.{n}.log"))
}

/// Moves `path` to `<stem>.1.log`, `.1` to `.2` and so on, dropping the oldest past [`KEEP`].
pub fn rotate(path: &Path) {
    let _ = fs::remove_file(numbered(path, KEEP));
    for n in (1..KEEP).rev() {
        let _ = fs::rename(numbered(path, n), numbered(path, n + 1));
    }
    let _ = fs::rename(path, numbered(path, 1));
}

/// A log file that rotates itself at [`MAX_BYTES`] and writes nothing while logging is off.
/// Lines are written whole: a write that would cross the cap goes into the next file.
pub struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    size: u64,
}

impl RotatingFile {
    /// `name` in the logs folder (`"utter.log"`). Nothing is opened until the first write.
    pub fn new(name: &str) -> Self {
        Self { path: dir().join(name), file: None, size: 0 }
    }

    fn open(&mut self) -> io::Result<&mut File> {
        if self.file.is_none() {
            fs::create_dir_all(dir())?;
            let file = OpenOptions::new().create(true).append(true).open(&self.path)?;
            self.size = file.metadata().map(|m| m.len()).unwrap_or(0);
            self.file = Some(file);
        }
        Ok(self.file.as_mut().expect("just opened"))
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !enabled() {
            // Turned off since the last write: let go of the file, and it goes too.
            if self.file.take().is_some() {
                remove_all();
            }
            return Ok(buf.len());
        }
        self.open()?;
        if self.size > 0 && self.size + buf.len() as u64 > MAX_BYTES {
            self.file = None;
            rotate(&self.path);
            self.open()?;
        }
        let file = self.file.as_mut().expect("open");
        file.write_all(buf)?;
        self.size += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), |f| f.flush())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_the_newest_files() {
        let tmp = std::env::temp_dir().join(format!("utter-logs-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("x.log");
        for round in 0..(KEEP + 2) {
            fs::write(&path, format!("{round}")).unwrap();
            rotate(&path);
        }
        assert!(!path.exists());
        for n in 1..=KEEP {
            // .1 is the newest round.
            let want = (KEEP + 2 - n).to_string();
            assert_eq!(fs::read_to_string(numbered(&path, n)).unwrap(), want);
        }
        assert!(!numbered(&path, KEEP + 1).exists());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
