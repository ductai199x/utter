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

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub fn free_bytes(_path: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_temp_drive_has_some_space() {
        let free = super::free_bytes(&std::env::temp_dir());
        if cfg!(windows) {
            assert!(free.unwrap() > 0);
        }
    }
}
