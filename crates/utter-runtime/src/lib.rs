//! Utter's runtime components: everything it runs on besides the GPU driver.
//!
//! `components.json` pins each component to a version and to files from its publisher, each
//! with the SHA-256 the publisher lists. A component is either one archive (`url`, `sha256`,
//! `size`, and `extract` globs naming the files to keep) or a list of single `files`.
//!
//! [`install`] downloads to `<data_dir>/downloads/` (archives) or straight into a staging
//! folder (files), verifies every byte, and only then renames the staging folder to
//! `<data_dir>/runtime/<id>-<version>/`: a component folder that exists is complete. Nothing
//! is loaded from anywhere else, in development or release, so Utter depends on no other
//! installed application.

pub mod gpu;

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One file from a publisher, pinned by size and SHA-256.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteFile {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedFile {
    /// The file's name in the component folder.
    pub name: String,
    #[serde(flatten)]
    pub file: RemoteFile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Source {
    /// One archive (zip or wheel); `extract` globs name the entries to keep, flattened.
    Archive {
        #[serde(flatten)]
        archive: RemoteFile,
        extract: Vec<String>,
    },
    /// Single files, each placed in the component folder under its name.
    Files { files: Vec<NamedFile> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub id: String,
    pub version: String,
    /// `std::env::consts::OS` value, or "any" for platform-independent files such as models.
    pub os: String,
    /// `std::env::consts::ARCH` value, or "any".
    pub arch: String,
    #[serde(flatten)]
    pub source: Source,
    pub license: String,
    pub purpose: String,
    /// Installed only on request (e.g. the quality-tier model); `utter-cli setup` skips it.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    components: Vec<Component>,
}

/// What [`install`] reports while it works.
#[derive(Debug, Clone, Copy)]
pub enum Progress {
    /// Bytes of the whole component.
    Downloading {
        done: u64,
        total: u64,
    },
    Verifying,
    Extracting,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("downloading {url}: {message}")]
    Download { url: String, message: String },
    #[error("{id}: checksum mismatch for {url}: expected {expected}, got {actual}")]
    Checksum { id: String, url: String, expected: String, actual: String },
    #[error("{id}: size mismatch for {url}: expected {expected} bytes, got {actual}")]
    Size { id: String, url: String, expected: u64, actual: u64 },
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

fn matches_platform(c: &Component) -> bool {
    (c.os == "any" || c.os == std::env::consts::OS) && (c.arch == "any" || c.arch == std::env::consts::ARCH)
}

/// The components for this platform, in manifest order.
pub fn components() -> &'static [Component] {
    static ALL: OnceLock<Vec<Component>> = OnceLock::new();
    ALL.get_or_init(|| {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../components.json")).expect("components.json is valid");
        manifest.components.into_iter().filter(matches_platform).collect()
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

    /// Bytes to download.
    pub fn download_size(&self) -> u64 {
        match &self.source {
            Source::Archive { archive, .. } => archive.size,
            Source::Files { files } => files.iter().map(|f| f.file.size).sum(),
        }
    }
}

/// Written last into a component folder: what was installed, from what.
const RECEIPT: &str = "utter-component.json";

#[derive(Serialize)]
struct Receipt<'a> {
    id: &'a str,
    version: &'a str,
    sources: Vec<&'a RemoteFile>,
    files: Vec<String>,
}

/// Downloads, verifies and installs `c`. A component already installed is left alone.
pub fn install(c: &Component, mut progress: impl FnMut(Progress)) -> Result<(), RuntimeError> {
    if c.is_installed() {
        return Ok(());
    }
    let final_dir = c.dir();
    let staging = staging_dir(&final_dir);
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io_err(&staging))?;
    }
    fs::create_dir_all(&staging).map_err(io_err(&staging))?;
    let total = c.download_size();

    let (files, sources, archive_to_remove) = match &c.source {
        Source::Archive { archive, extract } => {
            let downloads = downloads_dir();
            fs::create_dir_all(&downloads).map_err(io_err(&downloads))?;
            let file_name = archive.url.rsplit('/').next().unwrap_or(&c.id).to_owned();
            let path = downloads.join(&file_name);
            if !(path.is_file() && verify(&c.id, archive, &path, &mut progress).is_ok()) {
                download(archive, &path, 0, total, &mut progress)?;
                verify(&c.id, archive, &path, &mut progress)?;
            }
            progress(Progress::Extracting);
            let files = extract_archive(&c.id, extract, &path, &staging)?;
            (files, vec![archive], Some(path))
        }
        Source::Files { files } => {
            let mut done = 0u64;
            let mut names = Vec::new();
            for f in files {
                if f.name.contains(['/', '\\']) || f.name.starts_with('.') {
                    return Err(RuntimeError::Archive {
                        id: c.id.clone(),
                        message: format!("bad file name {}", f.name),
                    });
                }
                let path = staging.join(&f.name);
                download(&f.file, &path, done, total, &mut progress)?;
                verify(&c.id, &f.file, &path, &mut progress)?;
                done += f.file.size;
                names.push(f.name.clone());
            }
            names.sort();
            (names, files.iter().map(|f| &f.file).collect(), None)
        }
    };

    let receipt = Receipt { id: &c.id, version: &c.version, sources, files };
    let receipt_path = staging.join(RECEIPT);
    fs::write(&receipt_path, serde_json::to_string_pretty(&receipt).expect("receipt serializes"))
        .map_err(io_err(&receipt_path))?;
    fs::rename(&staging, &final_dir).map_err(io_err(&final_dir))?;
    if let Some(path) = archive_to_remove {
        fs::remove_file(&path).map_err(io_err(&path))?;
    }
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

/// Downloads `file` to `dest` (through `<dest>.part`). `offset` and `total` place this file's
/// bytes within the whole component, for progress.
fn download(
    file: &RemoteFile,
    dest: &Path,
    offset: u64,
    total: u64,
    progress: &mut impl FnMut(Progress),
) -> Result<(), RuntimeError> {
    let part = staging_dir(dest);
    let fail = |message: String| RuntimeError::Download { url: file.url.clone(), message };
    let response = ureq::get(&file.url).call().map_err(|e| fail(e.to_string()))?;
    let mut body = response.into_body().into_reader();
    let out_file = File::create(&part).map_err(io_err(&part))?;
    let mut out = BufWriter::with_capacity(1 << 20, out_file);
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = body.read(&mut buf).map_err(|e| fail(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io_err(&part))?;
        done += n as u64;
        progress(Progress::Downloading { done: offset + done, total });
    }
    out.flush().map_err(io_err(&part))?;
    drop(out);
    fs::rename(&part, dest).map_err(io_err(dest))
}

fn verify(id: &str, file: &RemoteFile, path: &Path, progress: &mut impl FnMut(Progress)) -> Result<(), RuntimeError> {
    progress(Progress::Verifying);
    let size = fs::metadata(path).map_err(io_err(path))?.len();
    if size != file.size {
        return Err(RuntimeError::Size { id: id.to_owned(), url: file.url.clone(), expected: file.size, actual: size });
    }
    let mut f = File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    let expected = file.sha256.trim_start_matches("sha256:");
    if actual != expected {
        return Err(RuntimeError::Checksum {
            id: id.to_owned(),
            url: file.url.clone(),
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(())
}

/// Extracts the archive entries matching `patterns`, flattened into `dest`. Fails if a
/// pattern matches nothing (the archive changed shape) or two entries share a file name.
fn extract_archive(id: &str, patterns: &[String], archive: &Path, dest: &Path) -> Result<Vec<String>, RuntimeError> {
    let fail = |message: String| RuntimeError::Archive { id: id.to_owned(), message };
    let globs: Vec<Glob> = patterns
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
        return Err(fail(format!("nothing matches {}", patterns[i])));
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

    fn check_file(id: &str, f: &RemoteFile) {
        assert!(f.url.starts_with("https://"), "{id}: {}", f.url);
        assert_eq!(f.sha256.len(), 64, "{id}: sha256 must be 64 hex digits");
        assert!(f.sha256.chars().all(|ch| ch.is_ascii_hexdigit()), "{id}");
        assert!(f.size > 0, "{id}");
    }

    #[test]
    fn every_component_is_pinned_and_checkable() {
        let all = all();
        assert!(!all.is_empty());
        for c in &all {
            match &c.source {
                Source::Archive { archive, extract } => {
                    check_file(&c.id, archive);
                    assert!(!extract.is_empty(), "{}", c.id);
                    for p in extract {
                        Glob::new(p).unwrap_or_else(|e| panic!("{}: pattern {p}: {e}", c.id));
                    }
                }
                Source::Files { files } => {
                    assert!(!files.is_empty(), "{}", c.id);
                    for f in files {
                        check_file(&c.id, &f.file);
                        assert!(!f.name.contains(['/', '\\']), "{}: {}", c.id, f.name);
                    }
                }
            }
        }
    }

    #[test]
    fn hugging_face_files_are_pinned_to_a_commit() {
        for c in all() {
            if let Source::Files { files } = &c.source {
                for f in files {
                    if f.file.url.contains("huggingface.co") {
                        let rev = f.file.url.split("/resolve/").nth(1).and_then(|r| r.split('/').next()).unwrap();
                        assert_eq!(rev.len(), 40, "{}: {} must name a commit, not a branch", c.id, f.file.url);
                    }
                }
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
