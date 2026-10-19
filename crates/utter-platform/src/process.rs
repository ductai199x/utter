//! Child processes that must not outlive Utter.
//!
//! `llama-server` holds gigabytes of GPU memory. If Utter crashes or is killed, nothing runs
//! its cleanup, so the operating system must end the child itself. On Windows every child
//! handed to [`tie_to_this_process`] joins one job object created with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when Utter's process ends, however it ends, its last
//! handle to the job closes and Windows terminates every process in the job.

use std::process::Child;

#[derive(Debug, thiserror::Error)]
#[error("tying the child process to Utter's lifetime: {0}")]
pub struct ProcessError(pub String);

/// Makes the operating system end `child` when this process ends.
pub fn tie_to_this_process(child: &Child) -> Result<(), ProcessError> {
    imp::tie(child)
}

/// Waits until process `pid` has ended, at most `timeout`; returns whether it did. A pid that
/// is not running (already gone) returns at once.
pub fn wait_for_exit(pid: u32, timeout: std::time::Duration) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        // SAFETY: OpenProcess returns a handle or null; the handle is closed after the wait.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return true;
            }
            let waited = WaitForSingleObject(handle, timeout.as_millis().min(u32::MAX as u128) as u32);
            CloseHandle(handle);
            waited != WAIT_TIMEOUT
        }
    }
    #[cfg(unix)]
    {
        let deadline = std::time::Instant::now() + timeout;
        // SAFETY: signal 0 only checks that the process exists.
        while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        true
    }
}

/// Starts the child without a console window of its own (Windows); a no-op elsewhere.
pub fn no_console_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{GetLastError, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    use super::ProcessError;

    /// The job's handle, kept open for the life of the process on purpose: closing it is what
    /// ends the children, and that must happen only when Utter itself ends.
    struct Job(HANDLE);
    // SAFETY: a job object handle may be used from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Result<Job, String>> = OnceLock::new();

    fn job() -> Result<HANDLE, ProcessError> {
        let job = JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with valid arguments; the handle is checked.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(format!("CreateJobObjectW failed: error {}", GetLastError()));
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    return Err(format!("SetInformationJobObject failed: error {}", GetLastError()));
                }
                Ok(Job(handle))
            }
        });
        job.as_ref().map(|j| j.0).map_err(|e| ProcessError(e.clone()))
    }

    pub fn tie(child: &Child) -> Result<(), ProcessError> {
        let job = job()?;
        // SAFETY: both handles are valid: the job is open for the process's life, and the
        // child's handle is owned by `child`, which the caller holds.
        let ok = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
        if ok == 0 {
            // SAFETY: reads the calling thread's last error.
            return Err(ProcessError(format!("AssignProcessToJobObject failed: error {}", unsafe { GetLastError() })));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::Child;

    use super::ProcessError;

    /// The tie is made as the child starts ([`super::spawn_tied`]), not after.
    pub fn tie(_child: &Child) -> Result<(), ProcessError> {
        Err(ProcessError("start it with process::spawn_tied".into()))
    }
}

/// Gives `cmd` the environment the desktop gave the AppImage, not the AppImage's own. Its
/// launcher points the library search path (and GTK's, GIO's, Python's...) into the image, and
/// children inherit it: llama-server took the image's libcrypto, built on Ubuntu 24.04, with
/// Ubuntu 26.04's libssl and did not start (2026-10-18). Every value naming the image ($APPDIR)
/// loses those entries, or goes when nothing is left; what the command sets itself is kept.
/// Nothing to do outside an AppImage, and off Linux.
pub fn outside_appimage(cmd: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        let Some(appdir) = std::env::var_os("APPDIR").filter(|d| !d.is_empty()) else { return };
        let appdir = std::path::PathBuf::from(appdir);
        let own: Vec<std::ffi::OsString> = cmd.get_envs().map(|(k, _)| k.to_owned()).collect();
        for (key, value) in std::env::vars_os() {
            if own.contains(&key) {
                continue;
            }
            if matches!(key.to_str(), Some("APPDIR" | "APPIMAGE" | "ARGV0" | "OWD" | "GTK_THEME")) {
                cmd.env_remove(&key);
                continue;
            }
            let entries: Vec<std::path::PathBuf> = std::env::split_paths(&value).collect();
            if !entries.iter().any(|p| p.starts_with(&appdir)) {
                continue;
            }
            let kept: Vec<_> =
                entries.into_iter().filter(|p| !p.starts_with(&appdir) && !p.as_os_str().is_empty()).collect();
            match std::env::join_paths(kept) {
                Ok(v) if !v.is_empty() => cmd.env(&key, v),
                _ => cmd.env_remove(&key),
            };
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cmd;
}

/// Starts `cmd` so that it ends when this process ends, even by a crash: Windows puts it in a
/// job object; Linux has the kernel kill it when its parent goes (`PR_SET_PDEATHSIG`); macOS has
/// no such tie (Utter sweeps a leftover server when it next starts, utter-llm).
pub fn spawn_tied(mut cmd: std::process::Command) -> std::io::Result<std::process::Child> {
    outside_appimage(&mut cmd);
    #[cfg(windows)]
    {
        let mut cmd = cmd;
        let child = cmd.spawn()?;
        if let Err(e) = tie_to_this_process(&child) {
            tracing::warn!("{e}: the child may outlive a crash of Utter");
        }
        Ok(child)
    }
    #[cfg(target_os = "linux")]
    {
        linux::spawn(cmd)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let mut cmd = cmd;
        cmd.spawn()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::sync::mpsc::{self, Sender};

    type Job = (Command, Sender<io::Result<Child>>);

    /// One thread that lives as long as the process spawns every tied child: the parent-death
    /// signal fires when the *thread* that forked the child exits, and Utter starts children
    /// from short-lived threads.
    fn spawner() -> &'static Mutex<Sender<Job>> {
        static SPAWNER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
        SPAWNER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("utter-spawner".into())
                .spawn(move || {
                    for (mut cmd, reply) in rx {
                        let parent = std::process::id() as libc::pid_t;
                        // SAFETY: the closure runs in the forked child before exec and calls
                        // only async-signal-safe functions (prctl, getppid).
                        unsafe {
                            cmd.pre_exec(move || {
                                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                                    return Err(io::Error::last_os_error());
                                }
                                // The parent may have died between fork and prctl.
                                if libc::getppid() != parent {
                                    return Err(io::Error::other("parent gone"));
                                }
                                Ok(())
                            });
                        }
                        let _ = reply.send(cmd.spawn());
                    }
                })
                .expect("spawn the process spawner");
            Mutex::new(tx)
        })
    }

    pub fn spawn(cmd: Command) -> io::Result<Child> {
        let (tx, rx) = mpsc::channel();
        spawner().lock().expect("spawner lock").send((cmd, tx)).map_err(|_| io::Error::other("spawner gone"))?;
        rx.recv().map_err(|_| io::Error::other("spawner gone"))?
    }
}
