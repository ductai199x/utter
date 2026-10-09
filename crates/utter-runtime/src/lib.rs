//! Utter's runtime components: everything it runs on besides the GPU driver.
//!
//! `components.json` pins each component to a version, a URL from its publisher and the
//! SHA-256 the publisher lists. [`install`] downloads one to `<data_dir>/downloads/`,
//! verifies it, extracts the files its `extract` globs name into a staging folder, and only
//! then renames the folder to `<data_dir>/runtime/<id>-<version>/`: a component folder that
//! exists is complete. Nothing is loaded from anywhere else, in development or release, so
//! Utter depends on no other installed application.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub id: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub extract: Vec<String>,
    pub license: String,
    pub purpose: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    components: Vec<Component>,
}

/// What [`install`] reports while it works.
#[derive(Debug, Clone, Copy)]
pub enum Progress {
    Downloading { done: u64, total: u64 },
    Verifying,
    Extracting,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("downloading {url}: {message}")]
    Download { url: String, message: String },
    #[error("{id}: checksum mismatch: expected {expected}, got {actual}")]
    Checksum { id: String, expected: String, actual: String },
    #[error("{id}: size mismatch: expected {expected} bytes, got {actual}")]
    Size { id: String, expected: u64, actual: u64 },
    #[error("{id}: archive: {message}")]
    Archive { id: String, message: String },
    #[error("unknown component {0}")]
    Unknown(String),
    #[error("{id} {version} is not installed: run `utter-cli setup`")]
    NotInstalled { id: String, version: String },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RuntimeError + '_ {
    move |source| RuntimeError::Io { path: path.to_path_buf(), source }
}

fn current_os() -> &'static str {
    std::env::consts::OS
}

fn current_arch() -> &'static str {
    std::env::consts::ARCH
}

/// The components for this platform, in manifest order.
pub fn components() -> &'static [Component] {
    static ALL: OnceLock<Vec<Component>> = OnceLock::new();
    ALL.get_or_init(|| {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../components.json")).expect("components.json is valid");
        manifest.components.into_iter().filter(|c| c.os == current_os() && c.arch == current_arch()).collect()
    })
}

pub fn component(id: &str) -> Result<&'static Component, RuntimeError> {
    components().iter().find(|c| c.id == id).ok_or_else(|| RuntimeError::Unknown(id.to_owned()))
}

pub fn runtime_dir() -> PathBuf {
    utter_core::data_dir().join("runtime")
}

fn downloads_dir() -> PathBuf {
    utter_core::data_dir().join("downloads")
}

impl Component {
    /// Where the component's files live once installed.
    pub fn dir(&self) -> PathBuf {
        runtime_dir().join(format!("{}-{}", self.id, self.version))
    }

    pub fn is_installed(&self) -> bool {
        self.dir().join(RECEIPT).is_file()
    }

    /// The installed component's folder, or an error naming how to install it.
    pub fn installed_dir(&self) -> Result<PathBuf, RuntimeError> {
        if self.is_installed() {
            Ok(self.dir())
        } else {
            Err(RuntimeError::NotInstalled { id: self.id.clone(), version: self.version.clone() })
        }
    }
}

/// Written last into a component folder: what was installed, from what.
const RECEIPT: &str = "utter-component.json";

#[derive(Serialize)]
struct Receipt<'a> {
    id: &'a str,
    version: &'a str,
    url: &'a str,
    sha256: &'a str,
    files: Vec<String>,
}

/// Downloads, verifies and extracts `c`. A component already installed is left alone.
pub fn install(c: &Component, mut progress: impl FnMut(Progress)) -> Result<(), RuntimeError> {
    if c.is_installed() {
        return Ok(());
    }
    let downloads = downloads_dir();
    fs::create_dir_all(&downloads).map_err(io_err(&downloads))?;
    let file_name = c.url.rsplit('/').next().unwrap_or(&c.id).to_owned();
    let archive = downloads.join(&file_name);
    if !(archive.is_file() && verify(c, &archive, &mut progress).is_ok()) {
        download(c, &archive, &mut progress)?;
        verify(c, &archive, &mut progress)?;
    }
    progress(Progress::Extracting);
    let final_dir = c.dir();
    let staging = staging_dir(&final_dir);
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
    }
    fs::create_dir_all(&staging).map_err(io_err(&staging))?;
    let files = extract(c, &archive, &staging)?;
    let receipt = Receipt { id: &c.id, version: &c.version, url: &c.url, sha256: &c.sha256, files };
    let receipt_path = staging.join(RECEIPT);
    fs::write(&receipt_path, serde_json::to_string_pretty(&receipt).expect("receipt serializes"))
        .map_err(io_err(&receipt_path))?;
    fs::rename(&staging, &final_dir).map_err(io_err(&final_dir))?;
    fs::remove_file(&archive).map_err(io_err(&archive))?;
    tracing::info!(id = %c.id, version = %c.version, dir = %final_dir.display(), "component installed");
    Ok(())
}

/// `<dir>.part`, appended rather than set with `with_extension`: versions have dots
/// ("cudnn-9.27.0.42"), and `with_extension` would replace the last one.
fn staging_dir(final_dir: &Path) -> PathBuf {
    let mut name = final_dir.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

fn download(c: &Component, dest: &Path, progress: &mut impl FnMut(Progress)) -> Result<(), RuntimeError> {
    let part = dest.with_extension("part");
    let fail = |message: String| RuntimeError::Download { url: c.url.clone(), message };
    let response = ureq::get(&c.url).call().map_err(|e| fail(e.to_string()))?;
    let mut body = response.into_body().into_reader();
    let file = File::create(&part).map_err(io_err(&part))?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = body.read(&mut buf).map_err(|e| fail(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io_err(&part))?;
        done += n as u64;
        progress(Progress::Downloading { done, total: c.size });
    }
    out.flush().map_err(io_err(&part))?;
    drop(out);
    fs::rename(&part, dest).map_err(io_err(dest))
}

fn verify(c: &Component, path: &Path, progress: &mut impl FnMut(Progress)) -> Result<(), RuntimeError> {
    progress(Progress::Verifying);
    let size = fs::metadata(path).map_err(io_err(path))?.len();
    if size != c.size {
        return Err(RuntimeError::Size { id: c.id.clone(), expected: c.size, actual: size });
    }
    let mut file = File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    let expected = c.sha256.trim_start_matches("sha256:");
    if actual != expected {
        return Err(RuntimeError::Checksum { id: c.id.clone(), expected: expected.to_owned(), actual });
    }
    Ok(())
}

/// Extracts the archive entries matching `c.extract`, flattened into `dest`. Fails if a
/// pattern matches nothing (the archive changed shape) or two entries share a file name.
fn extract(c: &Component, archive: &Path, dest: &Path) -> Result<Vec<String>, RuntimeError> {
    let fail = |message: String| RuntimeError::Archive { id: c.id.clone(), message };
    let globs: Vec<Glob> = c
        .extract
        .iter()
        .map(|p| Glob::new(p).map_err(|e| fail(format!("pattern {p}: {e}"))))
        .collect::<Result<_, _>>()?;
    let mut builder = GlobSetBuilder::new();
    for g in &globs {
        builder.add(g.clone());
    }
    let set: GlobSet = builder.build().map_err(|e| fail(e.to_string()))?;
    let file = File::open(archive).map_err(io_err(archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| fail(e.to_string()))?;
    let mut matched = vec![false; globs.len()];
    let mut written = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| fail(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        // Only the base name is used for the output path, so an archive entry can never
        // write outside `dest`.
        let name = entry.name().map_err(|e| fail(e.to_string()))?.into_owned();
        let hits = set.matches(&name);
        if hits.is_empty() {
            continue;
        }
        for h in hits {
            matched[h] = true;
        }
        let base = name.rsplit('/').next().unwrap_or(&name).to_owned();
        if written.contains(&base) {
            return Err(fail(format!("two entries named {base}")));
        }
        let out_path = dest.join(&base);
        let mut out = BufWriter::new(File::create(&out_path).map_err(io_err(&out_path))?);
        io::copy(&mut entry, &mut out).map_err(io_err(&out_path))?;
        out.flush().map_err(io_err(&out_path))?;
        written.push(base);
    }
    if let Some(i) = matched.iter().position(|m| !m) {
        return Err(fail(format!("nothing matches {}", c.extract[i])));
    }
    written.sort();
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Component> {
        let m: Manifest = serde_json::from_str(include_str!("../components.json")).unwrap();
        m.components
    }

    #[test]
    fn every_component_is_pinned_and_checkable() {
        let all = all();
        assert!(!all.is_empty());
        for c in &all {
            assert!(c.url.starts_with("https://"), "{}: {}", c.id, c.url);
            assert_eq!(c.sha256.len(), 64, "{}: sha256 must be 64 hex digits", c.id);
            assert!(c.sha256.chars().all(|ch| ch.is_ascii_hexdigit()), "{}", c.id);
            assert!(c.size > 0, "{}", c.id);
            assert!(!c.extract.is_empty(), "{}", c.id);
            for p in &c.extract {
                Glob::new(p).unwrap_or_else(|e| panic!("{}: pattern {p}: {e}", c.id));
            }
        }
    }

    #[test]
    fn ids_are_unique_per_platform() {
        let all = all();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!(a.id == b.id && a.os == b.os && a.arch == b.arch), "duplicate {}", a.id);
            }
        }
    }

    #[test]
    fn staging_keeps_every_dot_of_the_version() {
        let staged = staging_dir(Path::new("runtime/cudnn-9.27.0.42"));
        assert_eq!(staged, PathBuf::from("runtime/cudnn-9.27.0.42.part"));
    }
}
