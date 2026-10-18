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

mod backend;
pub mod gpu;

pub use backend::{Backend, backend};

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

    /// Installed, and every file the receipt lists still there (files downloaded as they are
    /// also at their size). A quick check, no hashing: it catches a deleted or truncated file,
    /// not one changed in place ([`Component::damaged_files`] hashes).
    pub fn is_intact(&self) -> bool {
        let dir = self.dir();
        let Ok(text) = fs::read_to_string(dir.join(RECEIPT)) else { return false };
        let Ok(receipt) = serde_json::from_str::<ReadReceipt>(&text) else { return false };
        let sizes: std::collections::HashMap<&str, u64> = match &self.source {
            Source::Files { files } => files.iter().map(|f| (f.name.as_str(), f.file.size)).collect(),
            Source::Archive { .. } => Default::default(),
        };
        receipt.files.iter().all(|name| match fs::metadata(dir.join(name)) {
            Ok(m) => sizes.get(name.as_str()).is_none_or(|&size| m.len() == size),
            Err(_) => false,
        })
    }

    /// The files of an installed component that fail their SHA-256 (only files downloaded as
    /// they are have one; an archive's extracted files are checked by [`Component::is_intact`]).
    pub fn damaged_files(&self) -> Vec<String> {
        let Source::Files { files } = &self.source else { return Vec::new() };
        let dir = self.dir();
        files
            .iter()
            .filter(|f| verify(&self.id, &f.file, &dir.join(&f.name), &mut |_| {}).is_err())
            .map(|f| f.name.clone())
            .collect()
    }

    /// Removes the installed component, so the next [`install`] fetches it again.
    pub fn uninstall(&self) -> Result<(), RuntimeError> {
        let dir = self.dir();
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(io_err(&dir))?;
        }
        Ok(())
    }

    /// The installed component's folder, or an error naming how to install it.
    /// Where the loaders find this component: this version's folder, or while an update to it
    /// downloads, the newest earlier version still whole ([`Component::previous_install`]), so
    /// dictation keeps running on it.
    pub fn installed_dir(&self) -> Result<PathBuf, RuntimeError> {
        if self.is_installed() {
            return Ok(self.dir());
        }
        match self.previous_install() {
            Some(dir) => {
                tracing::info!(id = %self.id, dir = %dir.display(), "an earlier version stands in until this one is installed");
                Ok(dir)
            }
            None => Err(RuntimeError::NotInstalled { id: self.id.clone(), version: self.version.clone() }),
        }
    }

    /// The newest other version of this component installed with every file of its receipt,
    /// if any: kept until this version is intact (prune), and used meanwhile.
    pub fn previous_install(&self) -> Option<PathBuf> {
        self.previous_install_in(&runtime_dir())
    }

    fn previous_install_in(&self, root: &Path) -> Option<PathBuf> {
        let current = root.join(format!("{}-{}", self.id, self.version));
        fs::read_dir(root)
            .ok()?
            .flatten()
            .filter(|e| e.path() != current && owner(&e.file_name().to_string_lossy()).is_some_and(|c| c.id == self.id))
            .filter_map(|e| {
                let dir = e.path();
                let receipt = fs::read_to_string(dir.join(RECEIPT)).ok()?;
                let receipt: ReadReceipt = serde_json::from_str(&receipt).ok()?;
                let whole = !receipt.files.is_empty() && receipt.files.iter().all(|f| dir.join(f).is_file());
                let installed = fs::metadata(dir.join(RECEIPT)).and_then(|m| m.modified()).ok()?;
                whole.then_some((installed, dir))
            })
            .max_by_key(|(installed, _)| *installed)
            .map(|(_, dir)| dir)
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

/// The part of a receipt read back: the files the install wrote.
#[derive(Deserialize)]
struct ReadReceipt {
    files: Vec<String>,
}

impl Component {
    /// Every file of the installed component (empty when it is not installed), for reading
    /// them ahead of the loaders ([`read_ahead`]).
    pub fn files_on_disk(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![self.dir()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                match e.file_type() {
                    Ok(t) if t.is_dir() => stack.push(e.path()),
                    Ok(_) => files.push(e.path()),
                    Err(_) => {}
                }
            }
        }
        files
    }
}

/// Reads `files` into the file cache ahead of the loaders, in the background, and returns at
/// once. On a cold start (after a reboot, or files not read for a while) the loaders pull them
/// in piecemeal: llama.cpp faults its mapped model in a few pages at a time, onnxruntime reads
/// in small pieces. Here the files are cut into [`READ_AHEAD_CHUNK`] ranges, in the order given
/// (what loads first comes first), and [`READ_AHEAD_THREADS`] threads read them at once.
///
/// First the biggest file's first range is timed: at memory speed the start is warm, and
/// nothing more is read (reading 7 GB from the cache again cost a warm start 0.3-0.6 s).
/// Measured (docs/eval-results.md, 2026-10-12): cold launch to ready 7.3 s -> 5.6-6.0 s, bound
/// by the disk (under 2 GB/s for these files on this machine, whatever the method: one
/// stream, eight, or the OS's own PrefetchVirtualMemory).
pub fn read_ahead(files: Vec<PathBuf>) {
    let _ = std::thread::Builder::new().name("utter-read-ahead".into()).spawn(move || read_ahead_now(files));
}

fn read_ahead_now(files: Vec<PathBuf>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let start = std::time::Instant::now();
    let sized: Vec<(PathBuf, u64)> =
        files.into_iter().filter_map(|p| fs::metadata(&p).ok().map(|m| (p, m.len()))).collect();
    let Some((big, big_len)) = sized.iter().max_by_key(|(_, len)| *len) else { return };
    let mut buf = vec![0u8; 4 << 20];
    let probe = READ_AHEAD_CHUNK.min(*big_len);
    let t = std::time::Instant::now();
    let probed = File::open(big).map(|f| read_range(&f, &mut buf, 0, probe)).unwrap_or(0);
    let rate = probed as f64 / t.elapsed().as_secs_f64().max(1e-6);
    if rate > CACHED_RATE {
        tracing::debug!(gb_s = rate / 1e9, "read ahead: the files are cached, nothing to read");
        return;
    }
    let mut chunks = std::collections::VecDeque::new();
    for (i, (path, len)) in sized.iter().enumerate() {
        let mut offset = if path == big { probe } else { 0 };
        while offset < *len {
            chunks.push_back((i, offset, READ_AHEAD_CHUNK.min(len - offset)));
            offset += READ_AHEAD_CHUNK;
        }
    }
    let chunks = std::sync::Mutex::new(chunks);
    let read = AtomicU64::new(probed);
    std::thread::scope(|scope| {
        for _ in 0..READ_AHEAD_THREADS {
            scope.spawn(|| {
                let mut buf = vec![0u8; 4 << 20];
                loop {
                    let Some((i, offset, len)) = chunks.lock().expect("read-ahead lock").pop_front() else { break };
                    if let Ok(f) = File::open(&sized[i].0) {
                        read.fetch_add(read_range(&f, &mut buf, offset, len), Ordering::Relaxed);
                    }
                }
            });
        }
    });
    tracing::debug!(mb = read.load(Ordering::Relaxed) >> 20, ms = start.elapsed().as_millis() as u64, "read ahead");
}

/// Reads `len` bytes of `f` from `offset` (fewer at its end); returns how many.
fn read_range(f: &File, buf: &mut [u8], offset: u64, len: u64) -> u64 {
    let mut done = 0;
    while done < len {
        let want = (len - done).min(buf.len() as u64) as usize;
        match read_at(f, &mut buf[..want], offset + done) {
            Ok(0) | Err(_) => break,
            Ok(n) => done += n as u64,
        }
    }
    done
}

/// The range each read-ahead read covers, how many threads read at once, and the rate (bytes a
/// second, one thread) above which a range came from the cache: the disk here gives about
/// 1.1 GB/s to one stream, the cache several times that.
const READ_AHEAD_CHUNK: u64 = 64 << 20;
const READ_AHEAD_THREADS: usize = 8;
const CACHED_RATE: f64 = 4e9;

/// One positional read (the file's cursor is not shared between threads).
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        f.seek_read(buf, offset)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        f.read_at(buf, offset)
    }
}

/// The component a runtime folder (`<id>-<version>`) belongs to: the longest id it is named
/// after ("parakeet-ultra-onnx-w8-..." is the w8 component's, not a shorter id's).
fn owner(folder: &str) -> Option<&'static Component> {
    components().iter().filter(|c| folder.starts_with(&format!("{}-", c.id))).max_by_key(|c| c.id.len())
}

/// Removes what an update left behind: component folders (and unfinished installs) of versions
/// components.json no longer lists, and downloads that are not current. Returns the bytes freed.
/// Only folders named `<known id>-<version>` are touched; anything else in the runtime folder
/// is left alone. An old version stays until the current one is intact: until then it is what
/// the update replaces (setup offers an update, not a repair).
pub fn prune() -> u64 {
    let current: std::collections::HashSet<std::ffi::OsString> = components()
        .iter()
        .flat_map(|c| {
            let dir = c.dir();
            [dir.file_name().map(|n| n.to_owned()), staging_dir(&dir).file_name().map(|n| n.to_owned())]
        })
        .flatten()
        .collect();
    let mut freed = 0;
    if let Ok(entries) = fs::read_dir(runtime_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let text = name.to_string_lossy();
            let replaced = owner(&text).is_some_and(|c| c.is_intact());
            if replaced && !current.contains(&name) && entry.path().is_dir() {
                let size = dir_size(&entry.path());
                match fs::remove_dir_all(entry.path()) {
                    Ok(()) => {
                        tracing::info!(dir = %entry.path().display(), bytes = size, "removed a superseded component");
                        freed += size;
                    }
                    Err(e) => tracing::warn!(dir = %entry.path().display(), "could not remove: {e}"),
                }
            }
        }
    }
    // Archives are deleted once extracted; any left over belong to no current install.
    let wanted: std::collections::HashSet<String> = components()
        .iter()
        .filter_map(|c| match &c.source {
            Source::Archive { archive, .. } => archive.url.rsplit('/').next().map(str::to_owned),
            Source::Files { .. } => None,
        })
        .flat_map(|name| [format!("{name}.part"), name])
        .collect();
    if let Ok(entries) = fs::read_dir(downloads_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !wanted.contains(&name) && entry.path().is_file() {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if fs::remove_file(entry.path()).is_ok() {
                    freed += size;
                }
            }
        }
    }
    freed
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

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
    // A staging folder left by an interrupted install is kept: its partial downloads resume.
    // Every file is verified by size and SHA-256 before it counts.
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
                // A file finished before an interruption is kept if it still verifies.
                if !(path.is_file() && verify(&c.id, &f.file, &path, &mut progress).is_ok()) {
                    download(&f.file, &path, done, total, &mut progress)?;
                    verify(&c.id, &f.file, &path, &mut progress)?;
                }
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
    // A folder left without its receipt (an install or removal cut short) would make the
    // rename fail after the whole download: it goes first. In use, this fails with the reason.
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(io_err(&final_dir))?;
    }
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
/// bytes within the whole component, for progress. A `.part` left by an interrupted download
/// is continued with a range request; a server that ignores the range sends the whole file,
/// and it starts over.
///
/// A connection that goes quiet (Wi-Fi changed, the computer slept) is an error after
/// [`STALL`], so Retry appears instead of a bar that never moves. A read cannot be
/// interrupted, so the body is copied on a thread of its own; one given up on never writes
/// again, and Retry continues the `.part` file.
fn download(
    file: &RemoteFile,
    dest: &Path,
    offset: u64,
    total: u64,
    progress: &mut impl FnMut(Progress),
) -> Result<(), RuntimeError> {
    let part = staging_dir(dest);
    let fail = |message: String| RuntimeError::Download { url: file.url.clone(), message };
    let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let have = if have < file.size { have } else { 0 };
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_connect(Some(STALL)).timeout_recv_response(Some(STALL)).build().into();
    let mut request = agent.get(&file.url);
    if have > 0 {
        request = request.header("Range", &format!("bytes={have}-"));
    }
    let response = request.call().map_err(|e| fail(e.to_string()))?;
    let resumed = have > 0 && response.status() == 206;
    let mut body = response.into_body().into_reader();
    // Unbuffered: a copy given up on must not flush anything when it ends.
    let mut out = if resumed {
        fs::OpenOptions::new().append(true).open(&part).map_err(io_err(&part))?
    } else {
        File::create(&part).map_err(io_err(&part))?
    };
    let given_up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<Result<usize, RuntimeError>>();
    {
        let given_up = given_up.clone();
        let written = part.clone();
        let url = file.url.clone();
        std::thread::Builder::new()
            .name("utter-download".into())
            .spawn(move || {
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let n = match body.read(&mut buf) {
                        Ok(n) => n,
                        Err(e) => {
                            let _ = tx.send(Err(RuntimeError::Download { url, message: e.to_string() }));
                            return;
                        }
                    };
                    if given_up.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    if n > 0
                        && let Err(e) = out.write_all(&buf[..n])
                    {
                        let _ = tx.send(Err(io_err(&written)(e)));
                        return;
                    }
                    if tx.send(Ok(n)).is_err() || n == 0 {
                        return;
                    }
                }
            })
            .map_err(io_err(&part))?;
    }
    let mut done = if resumed { have } else { 0 };
    loop {
        match rx.recv_timeout(STALL) {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                done += n as u64;
                progress(Progress::Downloading { done: offset + done, total });
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                given_up.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(fail(format!("no data for {} seconds", STALL.as_secs())));
            }
        }
    }
    fs::rename(&part, dest).map_err(io_err(dest))
}

/// How long a download may go without a byte before it is an error.
const STALL: std::time::Duration = std::time::Duration::from_secs(60);

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
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        return extract_tar(id, patterns, &set, archive, dest);
    }
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

/// [`extract_archive`] for a gzipped tar (the Linux and macOS builds). Shared libraries come as
/// a file and links to it under the names programs ask for (`libggml.so.0 -> libggml.so.0.26.0`):
/// a matching link becomes a link to its target's base name in the same flat folder (every
/// target in these archives sits beside its link), or on Windows a copy. Executable bits are
/// kept.
fn extract_tar(
    id: &str,
    patterns: &[String],
    set: &GlobSet,
    archive: &Path,
    dest: &Path,
) -> Result<Vec<String>, RuntimeError> {
    let fail = |message: String| RuntimeError::Archive { id: id.to_owned(), message };
    let file = File::open(archive).map_err(io_err(archive))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    let mut matched = vec![false; patterns.len()];
    let mut written = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    for entry in tar.entries().map_err(|e| fail(e.to_string()))? {
        let mut entry = entry.map_err(|e| fail(e.to_string()))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_symlink()) {
            continue;
        }
        let path = entry.path().map_err(|e| fail(e.to_string()))?.to_string_lossy().into_owned();
        let path = path.strip_prefix("./").unwrap_or(&path).to_owned();
        let hits = set.matches(&path);
        if hits.is_empty() {
            continue;
        }
        for h in hits {
            matched[h] = true;
        }
        let base = path.rsplit('/').next().unwrap_or(&path).to_owned();
        if written.contains(&base) || links.iter().any(|(b, _)| *b == base) {
            return Err(fail(format!("two entries named {base}")));
        }
        if kind.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|e| fail(e.to_string()))?
                .ok_or_else(|| fail(format!("{path}: link without a target")))?;
            let target = target.to_string_lossy();
            let target = target.rsplit('/').next().unwrap_or(&target).to_owned();
            links.push((base, target));
            continue;
        }
        let out_path = dest.join(&base);
        let mut out = BufWriter::new(File::create(&out_path).map_err(io_err(&out_path))?);
        io::copy(&mut entry, &mut out).map_err(io_err(&out_path))?;
        out.flush().map_err(io_err(&out_path))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = entry.header().mode().map_err(|e| fail(e.to_string()))? & 0o755;
            std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode | 0o644))
                .map_err(io_err(&out_path))?;
        }
        written.push(base);
    }
    // Links to links (libggml.so -> libggml.so.0 -> libggml.so.0.26.0): place each once its
    // target is there, in passes.
    while !links.is_empty() {
        let before = links.len();
        let mut later = Vec::new();
        for (base, target) in links {
            if !written.contains(&target) {
                later.push((base, target));
                continue;
            }
            let link = dest.join(&base);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link).map_err(io_err(&link))?;
            #[cfg(not(unix))]
            std::fs::copy(dest.join(&target), &link).map_err(io_err(&link))?;
            written.push(base);
        }
        if later.len() == before {
            let (base, target) = &later[0];
            return Err(fail(format!("{base} links to {target}, which is not extracted")));
        }
        links = later;
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

    /// A gzipped tar laid out as llama.cpp's Linux builds: a folder, a library with a chain of
    /// links to it, an executable, and a file not asked for.
    fn llama_like_tar(path: &Path) {
        let mut b =
            tar::Builder::new(flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::fast()));
        let mut file = |name: &str, body: &[u8], mode: u32| {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            b.append_data(&mut h, name, body).unwrap();
        };
        file("llama-b1/libggml.so.0.26.0", b"lib", 0o644);
        file("llama-b1/llama-server", b"#!exe", 0o755);
        file("llama-b1/llama-bench", b"skip", 0o755);
        for (name, target) in [("llama-b1/libggml.so", "libggml.so.0"), ("llama-b1/libggml.so.0", "libggml.so.0.26.0")]
        {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, name, target).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn a_tar_extracts_flat_with_its_link_chain_and_executable() {
        let dir = std::env::temp_dir().join(format!("utter-tar-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("out")).unwrap();
        let archive = dir.join("llama.tar.gz");
        llama_like_tar(&archive);
        let patterns = vec!["**/llama-server".to_owned(), "**/libggml.so*".to_owned()];
        let files = extract_archive("t", &patterns, &archive, &dir.join("out")).unwrap();
        assert_eq!(files, ["libggml.so", "libggml.so.0", "libggml.so.0.26.0", "llama-server"]);
        assert_eq!(fs::read(dir.join("out/libggml.so")).unwrap(), b"lib");
        assert!(!dir.join("out/llama-bench").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(fs::symlink_metadata(dir.join("out/libggml.so")).unwrap().file_type().is_symlink());
            assert_eq!(fs::metadata(dir.join("out/llama-server")).unwrap().permissions().mode() & 0o111, 0o111);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Serves `body` on localhost, honoring "Range: bytes=N-" when `ranges`; returns the URL
    /// and the Range headers it saw.
    fn serve(body: Vec<u8>, ranges: bool) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = io::BufReader::new(stream.try_clone().unwrap());
                let mut from = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        log.lock().unwrap().push(line.trim().to_owned());
                        if ranges {
                            from = v.trim().trim_end_matches('-').parse().unwrap();
                        }
                    }
                }
                let part = &body[from..];
                let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", part.len());
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(part).unwrap();
            }
        });
        (url, seen)
    }

    fn remote(url: &str, body: &[u8]) -> RemoteFile {
        RemoteFile { url: url.to_owned(), sha256: hex::encode(Sha256::digest(body)), size: body.len() as u64 }
    }

    #[test]
    fn an_interrupted_download_resumes_where_it_stopped() {
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        for ranges in [true, false] {
            let (url, seen) = serve(body.clone(), ranges);
            let dir = std::env::temp_dir().join(format!("utter-resume-{}-{ranges}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let dest = dir.join("file.bin");
            let _ = fs::remove_file(&dest);
            fs::write(staging_dir(&dest), &body[..40_000]).unwrap();
            let file = remote(&url, &body);
            let mut last = 0;
            download(&file, &dest, 0, file.size, &mut |p| {
                if let Progress::Downloading { done, .. } = p {
                    last = done;
                }
            })
            .unwrap();
            verify("test", &file, &dest, &mut |_| {}).unwrap();
            let seen: Vec<String> = seen.lock().unwrap().iter().map(|h| h.to_ascii_lowercase()).collect();
            assert_eq!(seen, ["range: bytes=40000-"]);
            assert_eq!(last, body.len() as u64, "progress counts what was there");
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_folder_belongs_to_the_longest_id_it_is_named_after() {
        assert_eq!(owner("parakeet-ultra-onnx-w8-0c71f044").map(|c| c.id.as_str()), Some("parakeet-ultra-onnx-w8"));
        assert_eq!(owner("utter-quality-dc947f0c").map(|c| c.id.as_str()), Some("utter-quality"));
        assert!(owner("notes").is_none());
    }

    #[test]
    fn an_earlier_version_with_all_its_files_stands_in() {
        let root = std::env::temp_dir().join(format!("utter-previous-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let c = component("utter-quality").unwrap();
        let install = |version: &str, files: &[&str], present: &[&str]| {
            let dir = root.join(format!("utter-quality-{version}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(RECEIPT), serde_json::json!({ "files": files }).to_string()).unwrap();
            present.iter().for_each(|f| fs::write(dir.join(f), b"x").unwrap());
            dir
        };
        assert_eq!(c.previous_install_in(&root), None);
        let whole = install("old", &["utter-quality-v1-q4_0.gguf"], &["utter-quality-v1-q4_0.gguf"]);
        // Newer, but a file is gone: not whole, so not used.
        std::thread::sleep(std::time::Duration::from_millis(20));
        install("broken", &["utter-quality-v1-q4_0.gguf"], &[]);
        // The current version's own folder is never an earlier one.
        install(&c.version, &["utter-quality-v2-q4_0.gguf"], &["utter-quality-v2-q4_0.gguf"]);
        assert_eq!(c.previous_install_in(&root), Some(whole));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "reads this machine's installed components"]
    fn installed_components_are_intact() {
        for c in components().iter().filter(|c| c.is_installed()) {
            assert!(c.is_intact(), "{} is installed but not intact", c.id);
            println!("{} {}: intact", c.id, c.version);
        }
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU"]
    fn the_gpu_is_probed() {
        let g = gpu::probe().unwrap();
        println!("{g:?}");
        assert!(g.memory_bytes > 1 << 30 && g.driver_cuda > 10_000 && !g.name.is_empty());
    }

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
