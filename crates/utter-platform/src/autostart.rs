//! Starting Utter when the user signs in (the Settings switch "Start with Windows").

use std::path::Path;

/// Registers `exe` to start at sign-in, or removes the registration.
pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
    imp::set(enabled, exe)
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
    };

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Utter";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: a NUL-terminated subkey name and an out-pointer for the key handle.
        let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), 0, KEY_SET_VALUE, &mut key) };
        if r != ERROR_SUCCESS {
            return Err(format!("open the Run key ({r})"));
        }
        let name = wide(NAME);
        let r = if enabled {
            // Quoted: the path may have spaces.
            let value = wide(&format!("\"{}\"", exe.display()));
            // SAFETY: an open key, a NUL-terminated name, and REG_SZ data with its byte length
            // (the terminating NUL included).
            unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32) }
        } else {
            // SAFETY: an open key and a NUL-terminated value name.
            match unsafe { RegDeleteValueW(key, name.as_ptr()) } {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                r => r,
            }
        };
        // SAFETY: the key opened above.
        unsafe { RegCloseKey(key) };
        if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("write the Run key ({r})")) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub fn set(_enabled: bool, _exe: &Path) -> Result<(), String> {
        Err("starting at sign-in is not supported on this platform yet".into())
    }
}
