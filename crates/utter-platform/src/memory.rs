//! The system's memory: what Metal (Apple Silicon's GPU shares it) and the CPU path run in.

/// Physical memory, in bytes.
pub fn total() -> Option<u64> {
    imp::total()
}

/// Memory available to start new work without swapping, in bytes, when the OS says.
pub fn available() -> Option<u64> {
    imp::available()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    fn status() -> Option<MEMORYSTATUSEX> {
        // SAFETY: a zeroed MEMORYSTATUSEX with its size set, as the call requires.
        let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: `s` is valid for writes.
        (unsafe { GlobalMemoryStatusEx(&mut s) } != 0).then_some(s)
    }

    pub fn total() -> Option<u64> {
        status().map(|s| s.ullTotalPhys)
    }

    pub fn available() -> Option<u64> {
        status().map(|s| s.ullAvailPhys)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    /// A `/proc/meminfo` field, in bytes (the file gives kB).
    fn meminfo(field: &str) -> Option<u64> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = text.lines().find(|l| l.starts_with(field) && l[field.len()..].starts_with(':'))?;
        line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
    }

    pub fn total() -> Option<u64> {
        meminfo("MemTotal")
    }

    pub fn available() -> Option<u64> {
        meminfo("MemAvailable")
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn total() -> Option<u64> {
        let mut value: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: a valid NUL-terminated name and an out-buffer of the size given.
        let rc = unsafe {
            libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut value).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        (rc == 0).then_some(value)
    }

    /// Not reported here: macOS keeps memory busy with caches it gives back on demand.
    pub fn available() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn total_memory_is_known_and_plausible() {
        let total = super::total().expect("total memory");
        assert!(total > 512 << 20, "{total}");
        if let Some(avail) = super::available() {
            assert!(avail <= total);
        }
    }
}
