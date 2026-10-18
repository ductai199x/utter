//! The processor: how many physical cores it has. Threads for compute-bound work go by physical
//! cores; a second hardware thread on a core (SMT) shares its arithmetic units and adds little.

/// Physical cores on this computer (at least 1). Falls back to the logical processors when the
/// OS will not say.
pub fn physical_cores() -> usize {
    imp::physical_cores()
        .filter(|&n| n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    /// One record per processor core (RelationProcessorCore), each of variable size.
    pub fn physical_cores() -> Option<usize> {
        let mut len = 0u32;
        // SAFETY: the first call only reports the buffer size; the second fills a buffer of that
        // size, aligned for the records, which are then walked by their own Size fields.
        unsafe {
            GetLogicalProcessorInformationEx(RelationProcessorCore, std::ptr::null_mut(), &mut len);
            if len == 0 {
                return None;
            }
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            let ptr = buf.as_mut_ptr().cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
            if GetLogicalProcessorInformationEx(RelationProcessorCore, ptr, &mut len) == 0 {
                return None;
            }
            let (mut offset, mut cores) = (0usize, 0usize);
            while offset < len as usize {
                let record =
                    &*(buf.as_ptr().cast::<u8>().add(offset).cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>());
                if record.Size == 0 {
                    break;
                }
                cores += 1;
                offset += record.Size as usize;
            }
            Some(cores)
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    /// Distinct (package, core) pairs in /proc/cpuinfo.
    pub fn physical_cores() -> Option<usize> {
        let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        let mut cores = std::collections::HashSet::new();
        let (mut package, mut core) = (None, None);
        for line in text.lines().chain(std::iter::once("")) {
            if line.trim().is_empty() {
                if let (Some(p), Some(c)) = (package.take(), core.take()) {
                    cores.insert((p, c));
                }
                continue;
            }
            let Some((key, value)) = line.split_once(':') else { continue };
            match key.trim() {
                "physical id" => package = value.trim().parse::<u32>().ok(),
                "core id" => core = value.trim().parse::<u32>().ok(),
                _ => {}
            }
        }
        // Some kernels (ARM, VMs) give no core ids: the caller falls back.
        (!cores.is_empty()).then_some(cores.len())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn physical_cores() -> Option<usize> {
        let mut n: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: sysctlbyname with a NUL-terminated name and an int-sized output buffer.
        let ok = unsafe {
            libc::sysctlbyname(
                c"hw.physicalcpu".as_ptr(),
                (&mut n as *mut libc::c_int).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        (ok && n > 0).then_some(n as usize)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn there_is_at_least_one_core_and_no_more_than_the_logical_processors() {
        let physical = super::physical_cores();
        let logical = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert!(physical >= 1 && physical <= logical, "{physical} physical, {logical} logical");
    }
}
