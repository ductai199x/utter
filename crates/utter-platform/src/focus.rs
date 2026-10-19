//! The window that has the keyboard focus: captured when the hotkey goes down, so the text
//! goes to that window and nowhere else (DESIGN.md section 4, step 2). Also which app owns it,
//! for paste rules and History (docs/ux/insertion.md), and whether Utter may send it keys.

/// A top-level window, as captured at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The OS handle (an HWND on Windows), as an integer.
    pub handle: isize,
    pub process_id: u32,
    pub title: String,
    /// The app that owns it (for a Store app, the app inside its frame host).
    pub app: App,
}

/// An app, by its program.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct App {
    /// The program's file name, stable across versions: "WINWORD.EXE", "mintty.exe". Empty
    /// when the OS would not say.
    pub exe: String,
    /// What a person calls it, from the program's version information ("Microsoft Word"); the
    /// file name without its extension when it has none.
    pub name: String,
}

/// The foreground window now, if any.
pub fn foreground() -> Option<Window> {
    imp::foreground()
}

/// The handle of a window the OS will not name (Linux, an app the accessibility bus does not
/// know): it is taken to be the one in front.
pub const UNKNOWN_WINDOW: isize = -1;

/// Whether `w` is still the foreground window (always, for an unknown one).
pub fn is_foreground(w: &Window) -> bool {
    w.handle == UNKNOWN_WINDOW || imp::foreground_handle() == Some(w.handle)
}

/// Whether the OS will drop key strokes Utter sends to `w`: on Windows, a window of a program
/// running as administrator while Utter is not (User Interface Privilege Isolation). `false`
/// when it cannot tell, so a paste is tried.
pub fn refuses_input(w: &Window) -> bool {
    imp::refuses_input(w.process_id)
}

/// Undoes what following the focus changed in the session, at quit (Linux: the accessibility
/// switch, if Utter turned it on; linux_a11y.rs).
pub fn shutdown() {
    #[cfg(target_os = "linux")]
    crate::linux_a11y::stop();
}

/// The apps with a visible window now, by name, each once: for "Add an app" (paste rules).
pub fn open_apps() -> Vec<App> {
    let mut apps = imp::open_apps();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by(|a, b| a.exe.eq_ignore_ascii_case(&b.exe));
    apps
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GW_OWNER, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };

    use super::{App, Window};

    /// Parts of Windows that can be in front without a text box to paste into.
    const NO_TEXT: &[&str] = &["TextInputHost.exe", "LockApp.exe"];
    /// The same, by window class: the taskbars (the tray menu was used) and the desktop.
    const NO_TEXT_CLASSES: &[&str] = &["Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Progman", "WorkerW"];

    /// A Store app's windows belong to this host; the app's own process is a child window's.
    const FRAME_HOST: &str = "ApplicationFrameHost.exe";
    /// Parts of Windows itself that own visible windows but are not apps a person dictates
    /// into (the frame host is here for a Store app whose own process is suspended).
    const SHELL: &[&str] = &[
        FRAME_HOST,
        "TextInputHost.exe",
        "ShellExperienceHost.exe",
        "StartMenuExperienceHost.exe",
        "SearchHost.exe",
        "LockApp.exe",
        "ShellHost.exe",
    ];

    pub fn foreground_handle() -> Option<isize> {
        // SAFETY: no arguments; returns null when no window has the focus.
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null()).then_some(hwnd as isize)
    }

    pub fn foreground() -> Option<Window> {
        // SAFETY: plain Win32 queries on a window handle just returned by the system; a handle
        // that has since closed makes them fail harmlessly.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let pid = window_pid(hwnd);
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let app_pid = store_app_pid(hwnd, pid).unwrap_or(pid);
            let app = app_of(app_pid);
            // Windows' own input overlay ("Windows Input Experience": the emoji, clipboard and
            // touch-keyboard panels) or the lock screen in front: no text box to paste into,
            // and a paste there vanishes. No window, so the text is copied instead.
            if NO_TEXT.iter().any(|exe| app.exe.eq_ignore_ascii_case(exe)) {
                return None;
            }
            // The taskbar or the desktop: a paste goes nowhere and would read as done.
            let mut class = [0u16; 64];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let class = String::from_utf16_lossy(&class[..len]);
            if NO_TEXT_CLASSES.contains(&class.as_str()) {
                return None;
            }
            Some(Window { handle: hwnd as isize, process_id: app_pid, title: String::from_utf16_lossy(&buf[..n]), app })
        }
    }

    pub fn refuses_input(pid: u32) -> bool {
        // SAFETY: the current process's pseudo handle needs no closing.
        let utter = unsafe { elevated(GetCurrentProcess()) };
        if utter == Some(true) {
            return false;
        }
        let Some(process) = Process::open(pid) else { return false };
        // SAFETY: a live process handle opened for limited queries.
        let elevated = unsafe { elevated(process.0) };
        elevated == Some(true)
    }

    pub fn open_apps() -> Vec<App> {
        let mut pids: Vec<u32> = Vec::new();
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: `lparam` is the Vec passed to EnumWindows below, alive for its duration.
            let pids = unsafe { &mut *(lparam as *mut Vec<u32>) };
            // SAFETY: plain queries on a window handle the enumeration just gave.
            unsafe {
                // An app's own windows: visible, titled, not owned by another window.
                if IsWindowVisible(hwnd) != 0 && GetWindowTextLengthW(hwnd) > 0 && GetWindow(hwnd, GW_OWNER).is_null() {
                    let pid = window_pid(hwnd);
                    pids.push(store_app_pid(hwnd, pid).unwrap_or(pid));
                }
            }
            1
        }
        // SAFETY: the callback only touches `pids` through `lparam` while EnumWindows runs.
        unsafe { EnumWindows(Some(each), &mut pids as *mut Vec<u32> as LPARAM) };
        pids.sort_unstable();
        pids.dedup();
        let own = std::process::id();
        pids.into_iter()
            .filter(|&p| p != own)
            .map(app_of)
            .filter(|a| !a.exe.is_empty() && !SHELL.iter().any(|s| s.eq_ignore_ascii_case(&a.exe)))
            .collect()
    }

    /// The process that owns `hwnd`.
    unsafe fn window_pid(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        // SAFETY (caller): `hwnd` is a window handle; a stale one leaves `pid` at 0.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    /// For a Store app's frame host window, the app's own process: the first child window that
    /// belongs to another process.
    unsafe fn store_app_pid(hwnd: HWND, host_pid: u32) -> Option<u32> {
        if !exe_path(host_pid).is_some_and(|p| file_name(&p).eq_ignore_ascii_case(FRAME_HOST)) {
            return None;
        }
        let mut found: (u32, u32) = (host_pid, 0);
        unsafe extern "system" fn each(child: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: `lparam` is the tuple passed to EnumChildWindows below.
            let found = unsafe { &mut *(lparam as *mut (u32, u32)) };
            // SAFETY: a child window handle the enumeration just gave.
            let pid = unsafe { window_pid(child) };
            if pid != 0 && pid != found.0 {
                found.1 = pid;
                return 0;
            }
            1
        }
        // SAFETY: the callback only touches `found` through `lparam` while this runs.
        unsafe { EnumChildWindows(hwnd, Some(each), &mut found as *mut (u32, u32) as LPARAM) };
        (found.1 != 0).then_some(found.1)
    }

    /// A process handle for limited queries, closed when dropped.
    struct Process(HANDLE);

    impl Process {
        fn open(pid: u32) -> Option<Self> {
            // SAFETY: plain call; null on failure (no such process, or no access).
            let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            (!h.is_null()).then_some(Self(h))
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: a handle this struct opened and owns.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Whether the process runs elevated (as administrator); `None` when it cannot be read.
    unsafe fn elevated(process: HANDLE) -> Option<bool> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY (caller): `process` is a valid handle with query access.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        // SAFETY: `info` is the TOKEN_ELEVATION buffer TokenElevation fills, sized below.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                &mut info as *mut TOKEN_ELEVATION as *mut core::ffi::c_void,
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
        };
        // SAFETY: the token handle opened above.
        unsafe { CloseHandle(token) };
        (ok != 0).then_some(info.TokenIsElevated != 0)
    }

    /// The full path of the process's program.
    fn exe_path(pid: u32) -> Option<String> {
        let process = Process::open(pid)?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` holds `len` UTF-16 units; the call writes at most that many.
        let ok = unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    fn file_name(path: &str) -> &str {
        path.rsplit(['\\', '/']).next().unwrap_or(path)
    }

    fn app_of(pid: u32) -> App {
        let Some(path) = exe_path(pid) else { return App::default() };
        let exe = file_name(&path).to_owned();
        let stem = exe.rsplit_once('.').map_or(exe.as_str(), |(s, _)| s).to_owned();
        // A description that is only the file name (some Store apps) says nothing more.
        let name = description(&path).filter(|d| !d.trim().is_empty() && !d.trim().eq_ignore_ascii_case(&exe));
        App { exe, name: name.unwrap_or_else(|| spaced(&stem)).trim().to_owned() }
    }

    /// A file name as words, for a program whose version information cannot be read (Store
    /// apps such as Windows Terminal): "WindowsTerminal" -> "Windows Terminal".
    pub(super) fn spaced(stem: &str) -> String {
        let mut out = String::with_capacity(stem.len() + 4);
        let mut prev: Option<char> = None;
        for c in stem.chars() {
            if c.is_uppercase() && prev.is_some_and(|p| p.is_lowercase()) {
                out.push(' ');
            }
            out.push(c);
            prev = Some(c);
        }
        out
    }

    /// The program's FileDescription (else ProductName) from its version information.
    fn description(path: &str) -> Option<String> {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut handle = 0u32;
        // SAFETY: `wide` is a NUL-terminated path.
        let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), &mut handle) };
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: `data` is `size` bytes, as GetFileVersionInfoSizeW asked for.
        if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
            return None;
        }
        let query = |sub: &str| -> Option<(*const u8, u32)> {
            let sub: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
            let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut len = 0u32;
            // SAFETY: `data` holds the version information block filled above.
            let ok = unsafe { VerQueryValueW(data.as_ptr().cast(), sub.as_ptr(), &mut ptr, &mut len) };
            (ok != 0 && !ptr.is_null() && len > 0).then_some((ptr as *const u8, len))
        };
        // The language and code page pairs the strings are stored under; English as a fallback.
        let mut keys = Vec::new();
        if let Some((ptr, len)) = query("\\VarFileInfo\\Translation") {
            for i in 0..(len as usize / 4) {
                // SAFETY: the Translation value is `len` bytes of (u16 language, u16 code page).
                let (lang, cp) = unsafe {
                    let p = ptr.add(i * 4) as *const u16;
                    (p.read_unaligned(), p.add(1).read_unaligned())
                };
                keys.push(format!("{lang:04x}{cp:04x}"));
            }
        }
        keys.extend(["040904b0".to_owned(), "040904e4".to_owned()]);
        let field = |name: &str| {
            keys.iter().find_map(|key| {
                let (ptr, len) = query(&format!("\\StringFileInfo\\{key}\\{name}"))?;
                // SAFETY: a string value is `len` UTF-16 units, including its NUL.
                let units = unsafe { std::slice::from_raw_parts(ptr as *const u16, len as usize) };
                let text = String::from_utf16_lossy(units).trim_end_matches('\0').trim().to_owned();
                (!text.is_empty()).then_some(text)
            })
        };
        let product = field("ProductName");
        match field("FileDescription") {
            // "Windows Terminal Host" is the program behind "Windows Terminal".
            Some(d) if product.as_deref().is_some_and(|p| d == format!("{p} Host")) => product,
            Some(d) => Some(d),
            None => product,
        }
    }
}

/// Linux, X11 and Wayland alike: the focused app from the accessibility bus (linux_a11y.rs),
/// its process as the window. Wayland tells a normal app nothing else about other windows. When
/// the accessibility bus knows nothing (an app with accessibility off, such as Chromium by
/// default), the focus is an unknown window that is always in front: the dictation is still
/// pasted where the person is typing, it only cannot be checked or matched to a paste rule.
#[cfg(target_os = "linux")]
mod imp {
    use super::{App, Window};
    use crate::linux_a11y;

    use super::UNKNOWN_WINDOW as UNKNOWN;

    pub fn foreground_handle() -> Option<isize> {
        Some(linux_a11y::focused().map_or(UNKNOWN, |f| f.pid as isize))
    }

    pub fn foreground() -> Option<Window> {
        let Some(f) = linux_a11y::focused() else {
            return Some(Window { handle: UNKNOWN, process_id: 0, title: String::new(), app: App::default() });
        };
        let (exe, path) = linux_a11y::process(f.pid).unwrap_or_default();
        Some(Window {
            handle: f.pid as isize,
            process_id: f.pid,
            title: exe.clone(),
            app: App { name: app_name(&exe, &path), exe },
        })
    }

    pub fn refuses_input(_pid: u32) -> bool {
        false
    }

    /// The apps on the accessibility bus that people see (not desktop services).
    pub fn open_apps() -> Vec<App> {
        linux_a11y::app_pids()
            .into_iter()
            .filter_map(linux_a11y::process)
            .filter(|(exe, _)| !is_service(exe))
            .map(|(exe, path)| App { name: app_name(&exe, &path), exe })
            .collect()
    }

    /// Desktop services on the accessibility bus, which own no window a person types into.
    fn is_service(exe: &str) -> bool {
        const PREFIXES: &[&str] =
            &["gsd-", "ibus", "xdg-desktop-portal", "evolution-", "gjs", "vmtoolsd", "vmware-", "at-spi"];
        PREFIXES.iter().any(|p| exe.starts_with(p))
            || matches!(
                exe,
                "gnome-shell" | "plasmashell" | "kded6" | "kwin_wayland" | "kwin_x11" | "xdg-permission-store"
            )
    }

    /// What people call an app: the Name of its desktop entry ("Text Editor" for
    /// gnome-text-editor), else the program's file name.
    fn app_name(exe: &str, _path: &str) -> String {
        name_for(desktop_names(), exe).unwrap_or_else(|| exe.to_owned())
    }

    /// The entry for `exe`, by its program (case aside), else by the longest program that `exe`
    /// extends with a dash: the process behind an app is often a helper of its launcher
    /// (gnome-terminal-server for gnome-terminal, ptyxis-agent for ptyxis).
    fn name_for(names: &std::collections::HashMap<String, String>, exe: &str) -> Option<String> {
        let exe = exe.to_lowercase();
        names.get(&exe).cloned().or_else(|| {
            names
                .iter()
                .filter(|(program, _)| exe.strip_prefix(program.as_str()).is_some_and(|rest| rest.starts_with('-')))
                .max_by_key(|(program, _)| program.len())
                .map(|(_, name)| name.clone())
        })
    }

    /// Program file name to the Name of the desktop entry that starts it (its Exec or TryExec
    /// program, or its StartupWMClass), from the system's, Flatpak's and the person's entries;
    /// read once.
    fn desktop_names() -> &'static std::collections::HashMap<String, String> {
        static NAMES: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();
        NAMES.get_or_init(|| {
            let mut dirs: Vec<std::path::PathBuf> = std::env::var("XDG_DATA_DIRS")
                .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
                .split(':')
                .map(std::path::PathBuf::from)
                .collect();
            dirs.push("/var/lib/flatpak/exports/share".into());
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                dirs.push(home.join(".local/share/flatpak/exports/share"));
                dirs.push(home.join(".local/share"));
            }
            let mut names = std::collections::HashMap::new();
            // Later directories win: the person's own entries over the system's.
            for dir in dirs {
                let Ok(entries) = std::fs::read_dir(dir.join("applications")) else { continue };
                for entry in entries.flatten() {
                    if entry.path().extension().is_some_and(|e| e == "desktop")
                        && let Ok(text) = std::fs::read_to_string(entry.path())
                    {
                        for (program, name) in desktop_entry(&text) {
                            names.insert(program.to_lowercase(), name.clone());
                        }
                    }
                }
            }
            names
        })
    }

    /// The programs a desktop entry names, each with the entry's Name.
    fn desktop_entry(text: &str) -> Vec<(String, String)> {
        let (mut name, mut programs, mut in_entry) = (None, Vec::new(), false);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            let Some((key, value)) = line.split_once('=').filter(|_| in_entry) else { continue };
            match key.trim() {
                "Name" => name = Some(value.trim().to_owned()),
                "Exec" | "TryExec" => {
                    // The program after any "env VAR=x"; Flatpak's entries start `flatpak run`,
                    // and their windows are matched by StartupWMClass instead.
                    let program = value.split_whitespace().find(|w| *w != "env" && !w.contains('='));
                    if let Some(base) = program.and_then(|p| p.rsplit('/').next()).filter(|b| *b != "flatpak") {
                        programs.push(base.trim_matches('"').to_owned());
                    }
                }
                "StartupWMClass" => programs.push(value.trim().to_owned()),
                _ => {}
            }
        }
        name.map(|n| programs.into_iter().map(|p| (p, n.clone())).collect()).unwrap_or_default()
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_desktop_entry_names_its_programs() {
            let entry = "[Desktop Entry]\nName=Text Editor\nExec=gnome-text-editor %U\nTryExec=gnome-text-editor\n\
                         [Desktop Action new-window]\nName=New Window\nExec=gnome-text-editor --new-window\n";
            let found = super::desktop_entry(entry);
            assert!(found.iter().all(|(p, n)| p == "gnome-text-editor" && n == "Text Editor"));
            assert_eq!(found.len(), 2);
            let env = "[Desktop Entry]\nName=Thing\nExec=env FOO=1 /opt/thing/bin/thing-app --x\n";
            assert_eq!(super::desktop_entry(env), [("thing-app".to_owned(), "Thing".to_owned())]);
        }

        #[test]
        fn a_helper_process_takes_its_launcher_s_name() {
            let names: std::collections::HashMap<String, String> =
                [("gnome-terminal", "Terminal"), ("gnome", "Wrong"), ("firefox", "Firefox")]
                    .into_iter()
                    .map(|(a, b)| (a.to_owned(), b.to_owned()))
                    .collect();
            assert_eq!(super::name_for(&names, "gnome-terminal-server").as_deref(), Some("Terminal"));
            assert_eq!(super::name_for(&names, "Firefox").as_deref(), Some("Firefox"));
            assert_eq!(super::name_for(&names, "firefoxish"), None);
        }

        #[test]
        fn services_are_not_apps() {
            for exe in ["evolution-alarm-notify", "xdg-desktop-portal-gtk", "gjs-console", "vmtoolsd", "gsd-media-keys"]
            {
                assert!(super::is_service(exe), "{exe}");
            }
            assert!(!super::is_service("gnome-text-editor"));
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub use crate::macos::focus::*;
}

#[cfg(test)]
mod tests {
    /// Prints what Utter sees on this machine: run by hand with --ignored --nocapture.
    #[test]
    #[ignore]
    fn show_the_foreground_window_and_open_apps() {
        println!("foreground: {:?}", super::foreground());
        if let Some(w) = super::foreground() {
            println!("refuses input: {}", super::refuses_input(&w));
        }
        for app in super::open_apps() {
            println!("{:<28} {}", app.exe, app.name);
        }
    }

    #[test]
    #[cfg(windows)]
    fn a_file_name_reads_as_words() {
        assert_eq!(super::imp::spaced("WindowsTerminal"), "Windows Terminal");
        assert_eq!(super::imp::spaced("Notepad"), "Notepad");
        assert_eq!(super::imp::spaced("msedge"), "msedge");
        assert_eq!(super::imp::spaced("OUTLOOK"), "OUTLOOK");
    }
}
