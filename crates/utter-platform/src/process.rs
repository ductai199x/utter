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

/// Starts `cmd` so that it ends when this process ends, even by a crash: Windows puts it in a
/// job object; Linux has the kernel kill it when its parent goes (`PR_SET_PDEATHSIG`); macOS has
/// no such tie (Utter sweeps a leftover server when it next starts, utter-llm).
pub fn spawn_tied(cmd: std::process::Command) -> std::io::Result<std::process::Child> {
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
