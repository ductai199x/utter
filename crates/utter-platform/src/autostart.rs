//! Starting Utter when the user signs in (the Settings switch "Start with Windows", "Start at
//! login" elsewhere).

use std::path::Path;

/// Registers `exe` to start at sign-in with `arg` (so it can tell a sign-in start from the
/// person starting it), or removes the registration.
pub fn set(enabled: bool, exe: &Path, arg: &str) -> Result<(), String> {
    imp::set(enabled, exe, arg)
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

    pub fn set(enabled: bool, exe: &Path, arg: &str) -> Result<(), String> {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: a NUL-terminated subkey name and an out-pointer for the key handle.
        let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), 0, KEY_SET_VALUE, &mut key) };
        if r != ERROR_SUCCESS {
            return Err(format!("open the Run key ({r})"));
        }
        let name = wide(NAME);
        let r = if enabled {
            // The path quoted (it may have spaces), then the argument.
            let value = wide(&format!("\"{}\" {arg}", exe.display()));
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

/// Linux: a desktop entry in the XDG autostart folder, which GNOME, KDE and the other desktops
/// start at sign-in.
#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::path::{Path, PathBuf};

    fn entry() -> Result<PathBuf, String> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or("no home folder")?;
        Ok(config.join("autostart").join("utter.desktop"))
    }

    pub fn set(enabled: bool, exe: &Path, arg: &str) -> Result<(), String> {
        let path = entry()?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
                _ => Ok(()),
            };
        }
        let dir = path.parent().ok_or("autostart folder")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // From an AppImage the program runs from a mount made anew at every start: the image
        // itself is what starts it ($APPIMAGE, set by its runtime).
        let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or_else(|| exe.to_path_buf());
        // Exec quoting (Desktop Entry spec): the path in double quotes, \ " ` $ escaped.
        let quoted = exe
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$");
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Utter\nComment=Voice dictation\nExec=\"{quoted}\" {arg}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
        );
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// macOS: a LaunchAgent, run at sign-in (a login item through SMAppService needs a signed app
/// bundle; Utter ships through Homebrew, unsigned).
#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};

    fn agent() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("no home folder")?;
        Ok(PathBuf::from(home).join("Library/LaunchAgents/app.utter.plist"))
    }

    fn xml(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn set(enabled: bool, exe: &Path, arg: &str) -> Result<(), String> {
        let path = agent()?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
                _ => Ok(()),
            };
        }
        let dir = path.parent().ok_or("LaunchAgents folder")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>app.utter</string>\n<key>ProgramArguments</key><array><string>{}</string><string>{}</string></array>\n<key>RunAtLoad</key><true/>\n<key>ProcessType</key><string>Interactive</string>\n</dict></plist>\n",
            xml(&exe.display().to_string()),
            xml(arg)
        );
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}
