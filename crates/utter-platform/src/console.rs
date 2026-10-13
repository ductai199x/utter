//! The terminal that started Utter, for `--version` and `--help`: a release build has no console
//! of its own (it is a windowed app), so what it prints goes nowhere until it attaches to its
//! parent's.

/// Sends stdout and stderr to the console of the process that started this one, if it has one.
/// Returns whether one was attached; elsewhere than Windows, output reaches the terminal anyway.
pub fn attach_parent() -> bool {
    imp::attach_parent()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

    pub fn attach_parent() -> bool {
        // SAFETY: no pointers; fails harmlessly when there is no parent console or one is
        // already attached (a debug build is a console app).
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn attach_parent() -> bool {
        true
    }
}
