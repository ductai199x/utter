//! Free space on the drive that holds a path (first run checks it before downloading).

use std::path::Path;

/// Bytes free for this user on the drive holding `path` (which must exist), or `None` when the
/// platform cannot say.
pub fn free_bytes(path: &Path) -> Option<u64> {
    imp::free_bytes(path)
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    pub fn free_bytes(path: &Path) -> Option<u64> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        // SAFETY: a NUL-terminated wide path, and one out-pointer to a u64; the others may be
        // null.
        let ok =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some(available)
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// The space available to unprivileged users (f_bavail), as `df` shows it. A folder that
    /// does not exist yet is measured at its nearest existing parent.
    pub fn free_bytes(path: &Path) -> Option<u64> {
        let existing = path.ancestors().find(|p| p.exists())?;
        let c = CString::new(existing.as_os_str().as_bytes()).ok()?;
        // SAFETY: a valid NUL-terminated path and a zeroed struct for the call to fill.
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)] // the field types differ between Linux and macOS
        Some(s.f_bavail as u64 * s.f_frsize as u64)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_temp_drive_has_some_space() {
        let free = super::free_bytes(&std::env::temp_dir());
        assert!(free.unwrap() > 0);
    }
}
