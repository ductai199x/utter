//! Putting text into the focused app (DESIGN.md section 4, step 8; docs/ux/insertion.md).
//!
//! [`paste`]: save the clipboard, put the text on it, press the app's paste keys, and restore
//! the clipboard once the app has had time to read it. Rules:
//! - nothing is typed unless the window captured at key down still has the focus;
//! - no keys go to a window the OS would drop them for (an administrator app, focus.rs);
//! - every clipboard format held in global memory is saved and put back (formats that are GDI
//!   handles, such as CF_BITMAP, cannot be copied; their memory-based equivalents, such as
//!   CF_DIB, are kept);
//! - the clipboard is restored only if nothing else has changed it since the paste, so a copy
//!   the user makes in the meantime is never overwritten;
//! - the dictated text is marked to stay out of Windows' clipboard history and cloud clipboard.
//!
//! How an app pastes, and how long it needs the text on the clipboard, is its [`PasteRule`]:
//! this platform's defaults ([`default_rules`]) with the person's changes over them.
//! [`copy`] leaves text on the clipboard: the fallback when it cannot go in.

use std::time::Duration;

use crate::focus::{self, Window};

#[derive(Debug, thiserror::Error)]
pub enum InsertError {
    #[error("the focus moved away from \"{0}\"; nothing was typed")]
    FocusChanged(String),
    #[error("\"{0}\" runs as administrator; Windows drops keys sent to it")]
    Refused(String),
    #[error("clipboard: {0}")]
    Clipboard(String),
    #[error("sending keys: {0}")]
    Input(String),
}

/// The keys that paste in an app, or typing the text out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKeys {
    /// Ctrl+V (Cmd+V on macOS): nearly every app.
    Primary,
    /// Ctrl+Shift+V: Linux terminals and some Windows ones.
    CtrlShiftV,
    /// Shift+Insert: old terminals, PuTTY, mintty.
    ShiftInsert,
    /// The text as key strokes, for apps that block pasting. A line break is an Enter key.
    Type,
}

/// How long the dictation stays on the clipboard before the old contents come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// 0.5 s: enough for apps that read the clipboard at once.
    Normal,
    /// For apps that read it late (Word, Outlook).
    Seconds3,
    /// For remote desktops, which sync it over the network.
    Seconds5,
    /// The old clipboard is not restored.
    Leave,
}

impl Keep {
    /// When to restore the old clipboard; `None`: never.
    pub fn restore_after(self) -> Option<Duration> {
        match self {
            Keep::Normal => Some(Duration::from_millis(500)),
            Keep::Seconds3 => Some(Duration::from_secs(3)),
            Keep::Seconds5 => Some(Duration::from_secs(5)),
            Keep::Leave => None,
        }
    }
}

/// How one app pastes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteRule {
    pub keys: PasteKeys,
    pub keep: Keep,
}

impl Default for PasteRule {
    fn default() -> Self {
        Self { keys: PasteKeys::Primary, keep: Keep::Normal }
    }
}

/// A default paste rule: the program's file name, what people call it, and the rule.
pub type DefaultRule = (&'static str, &'static str, PasteRule);

/// This platform's paste rules for apps that do not paste with the primary keys at once
/// (docs/ux/insertion.md, "Utter's defaults"). Every other app: [`PasteRule::default`].
pub fn default_rules() -> &'static [DefaultRule] {
    imp::DEFAULT_RULES
}

/// The primary paste keys as this platform writes them, for messages: "Ctrl+V" ("Cmd+V" on
/// macOS).
pub fn primary_keys() -> &'static str {
    if cfg!(target_os = "macos") { "Cmd+V" } else { "Ctrl+V" }
}

/// Pastes `text` into `target` by `rule`. Returns once the keys are sent; the clipboard is
/// restored on a background thread when the rule says.
pub fn paste(target: &Window, text: &str, rule: PasteRule) -> Result<(), InsertError> {
    if !focus::is_foreground(target) {
        return Err(InsertError::FocusChanged(target.title.clone()));
    }
    if focus::refuses_input(target) {
        return Err(InsertError::Refused(target.app.name.clone()));
    }
    match rule.keys {
        PasteKeys::Type => imp::type_text(text),
        keys => imp::paste(text, keys, rule.keep.restore_after()),
    }
}

/// The clipboard's text, if it holds any.
pub fn clipboard_text() -> Option<String> {
    imp::clipboard_text()
}

/// Puts `text` on the clipboard and leaves it there (marked to stay out of clipboard history,
/// as a paste is): for a dictation that could not go in, and for "Copy" in History.
pub fn copy(text: &str) -> Result<(), InsertError> {
    imp::put_text(text).map(|_| ())
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Ole::{
        CF_BITMAP, CF_DSPBITMAP, CF_DSPENHMETAFILE, CF_DSPMETAFILEPICT, CF_ENHMETAFILE, CF_METAFILEPICT,
        CF_OWNERDISPLAY, CF_PALETTE, CF_UNICODETEXT,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        SendInput, VK_CONTROL, VK_INSERT, VK_RETURN, VK_SHIFT,
    };

    use super::{DefaultRule, InsertError, Keep, PasteKeys, PasteRule};

    const fn rule(keys: PasteKeys, keep: Keep) -> PasteRule {
        PasteRule { keys, keep }
    }

    /// docs/ux/insertion.md, "Utter's defaults (Windows)".
    pub const DEFAULT_RULES: &[DefaultRule] = &[
        ("mintty.exe", "Git Bash (mintty)", rule(PasteKeys::ShiftInsert, Keep::Normal)),
        ("putty.exe", "PuTTY", rule(PasteKeys::ShiftInsert, Keep::Normal)),
        ("alacritty.exe", "Alacritty", rule(PasteKeys::CtrlShiftV, Keep::Normal)),
        ("wezterm-gui.exe", "WezTerm", rule(PasteKeys::CtrlShiftV, Keep::Normal)),
        ("WINWORD.EXE", "Microsoft Word", rule(PasteKeys::Primary, Keep::Seconds3)),
        ("OUTLOOK.EXE", "Outlook (classic)", rule(PasteKeys::Primary, Keep::Seconds3)),
        ("olk.exe", "Outlook", rule(PasteKeys::Primary, Keep::Seconds3)),
        ("WhatsApp.exe", "WhatsApp", rule(PasteKeys::Primary, Keep::Seconds3)),
        ("mstsc.exe", "Remote Desktop", rule(PasteKeys::Primary, Keep::Seconds5)),
        ("msrdc.exe", "Remote Desktop", rule(PasteKeys::Primary, Keep::Seconds5)),
        ("wfica32.exe", "Citrix Workspace", rule(PasteKeys::Primary, Keep::Seconds5)),
        ("CDViewer.exe", "Citrix Workspace", rule(PasteKeys::Primary, Keep::Seconds5)),
        ("TeamViewer.exe", "TeamViewer", rule(PasteKeys::Primary, Keep::Seconds5)),
        ("vncviewer.exe", "VNC Viewer", rule(PasteKeys::Primary, Keep::Seconds5)),
    ];

    /// Formats whose data is a GDI or other non-memory handle: they cannot be copied.
    const HANDLE_FORMATS: &[u16] = &[
        CF_BITMAP,
        CF_ENHMETAFILE,
        CF_METAFILEPICT,
        CF_PALETTE,
        CF_OWNERDISPLAY,
        CF_DSPBITMAP,
        CF_DSPENHMETAFILE,
        CF_DSPMETAFILEPICT,
    ];

    struct Clipboard;

    impl Clipboard {
        fn open() -> Result<Self, InsertError> {
            // SAFETY: null owner window; paired with CloseClipboard in Drop.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                Ok(Clipboard)
            } else {
                Err(InsertError::Clipboard(format!("busy: {}", std::io::Error::last_os_error())))
            }
        }
    }

    /// Runs a whole clipboard operation (open, act, close) until it works or `budget` runs out.
    /// Clipboard managers (Windows' own history, third-party ones) open the clipboard on every
    /// change to read it, so an operation right after a change can find it held, or lose it
    /// mid-way (EmptyClipboard: "Thread does not have a clipboard open"): both pass in moments.
    fn retrying<T>(budget: Duration, mut op: impl FnMut() -> Result<T, InsertError>) -> Result<T, InsertError> {
        let start = std::time::Instant::now();
        let mut wait = Duration::from_millis(5);
        loop {
            match op() {
                Ok(v) => return Ok(v),
                Err(e) if start.elapsed() + wait > budget => return Err(e),
                Err(_) => {
                    std::thread::sleep(wait);
                    wait = (wait * 2).min(Duration::from_millis(50));
                }
            }
        }
    }

    /// How long a paste's clipboard work may wait for another app to let go.
    const BUDGET: Duration = Duration::from_millis(600);
    /// The restore runs after the paste, in the background: it can wait longer.
    const RESTORE_BUDGET: Duration = Duration::from_millis(1500);

    impl Drop for Clipboard {
        fn drop(&mut self) {
            // SAFETY: the clipboard is open (this value exists only after OpenClipboard).
            unsafe { CloseClipboard() };
        }
    }

    fn register(name: &str) -> u32 {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated UTF-16 string.
        unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
    }

    /// Copies a global memory block's bytes.
    unsafe fn read_global(handle: HANDLE) -> Option<Vec<u8>> {
        // SAFETY (caller): `handle` is a clipboard data handle in global memory.
        unsafe {
            let size = GlobalSize(handle);
            let ptr = GlobalLock(handle) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            GlobalUnlock(handle);
            Some(bytes)
        }
    }

    /// Sets clipboard data from bytes (the clipboard must be open). On success the system
    /// owns the memory; on failure it is freed here.
    fn set_bytes(format: u32, bytes: &[u8]) -> bool {
        // SAFETY: allocates and fills a block of exactly `bytes.len()` (at least 1) bytes.
        unsafe {
            let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
            if mem.is_null() {
                return false;
            }
            let ptr = GlobalLock(mem) as *mut u8;
            if ptr.is_null() {
                GlobalFree(mem);
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            GlobalUnlock(mem);
            if SetClipboardData(format, mem as HANDLE).is_null() {
                GlobalFree(mem);
                return false;
            }
            true
        }
    }

    /// Everything on the clipboard, to put back after the paste. Contents can show up a moment
    /// after a change (measured: none at once, the text listed but unreadable at 20 ms, all of
    /// it by 100 ms, with a clipboard manager running): a format listed but not readable yet is
    /// waited for, and a clipboard that looks empty is looked at again briefly before it is
    /// believed, so a fresh copy is not saved as nothing and then wiped by the restore.
    pub(super) fn save_all() -> Result<Vec<(u32, Vec<u8>)>, InsertError> {
        let start = std::time::Instant::now();
        loop {
            // A format that never becomes readable (its app has closed) must not stop the
            // paste: after the wait, what can be read is saved.
            let saved = retrying(BUDGET, || save_all_once(true)).or_else(|_| save_all_once(false))?;
            if !saved.is_empty() || start.elapsed() >= EMPTY_SETTLE {
                return Ok(saved);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// The longest a paste waits for the dictation to read back from the clipboard.
    const READBACK: Duration = Duration::from_millis(200);

    /// How long an empty-looking clipboard is watched before it counts as empty.
    const EMPTY_SETTLE: Duration = Duration::from_millis(150);

    /// One pass over the formats. `strict`: a format listed but not readable yet fails the pass,
    /// to be tried again; otherwise it is left out.
    fn save_all_once(strict: bool) -> Result<Vec<(u32, Vec<u8>)>, InsertError> {
        let _cb = Clipboard::open()?;
        let mut saved = Vec::new();
        let mut format = 0u32;
        loop {
            // SAFETY: the clipboard is open; 0 starts the enumeration, 0 ends it.
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if HANDLE_FORMATS.contains(&(format as u16)) {
                continue;
            }
            // SAFETY: the clipboard is open and `format` is on it.
            let handle = unsafe { GetClipboardData(format) };
            if handle.is_null() {
                if strict {
                    return Err(InsertError::Clipboard(format!("format {format} is not ready")));
                }
                tracing::debug!("clipboard format {format} could not be read; it will not be put back");
                continue;
            }
            // SAFETY: every format not in HANDLE_FORMATS holds global memory.
            if let Some(bytes) = unsafe { read_global(handle) } {
                saved.push((format, bytes));
            }
        }
        Ok(saved)
    }

    pub fn clipboard_text() -> Option<String> {
        retrying(BUDGET, || {
            let _cb = Clipboard::open()?;
            // SAFETY: the clipboard is open; CF_UNICODETEXT holds global memory.
            let handle = unsafe { GetClipboardData(CF_UNICODETEXT as u32) };
            if handle.is_null() {
                return Ok(None);
            }
            // SAFETY: a clipboard data handle in global memory.
            let Some(bytes) = (unsafe { read_global(handle) }) else { return Ok(None) };
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Ok(Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned()))
        })
        .ok()
        .flatten()
    }

    pub fn put_text(text: &str) -> Result<u32, InsertError> {
        retrying(BUDGET, || put_text_once(text))
    }

    /// Puts `text` on the clipboard; returns the sequence number it leaves, read after the
    /// clipboard is closed (it changes on close: read while open it is one behind, and the
    /// restore would take the paste itself for the person's copy and never put anything back).
    fn put_text_once(text: &str) -> Result<u32, InsertError> {
        {
            let _cb = Clipboard::open()?;
            // SAFETY: the clipboard is open.
            if unsafe { EmptyClipboard() } == 0 {
                return Err(InsertError::Clipboard(format!("empty: {}", std::io::Error::last_os_error())));
            }
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let bytes: Vec<u8> = wide.iter().flat_map(|u| u.to_le_bytes()).collect();
            if !set_bytes(CF_UNICODETEXT as u32, &bytes) {
                return Err(InsertError::Clipboard("could not set the text".into()));
            }
            // Keep dictations out of clipboard history, cloud clipboard and clipboard monitors.
            set_bytes(register("ExcludeClipboardContentFromMonitorProcessing"), &[0]);
            set_bytes(register("CanIncludeInClipboardHistory"), &0u32.to_le_bytes());
            set_bytes(register("CanUploadToCloudClipboard"), &0u32.to_le_bytes());
        }
        // SAFETY: no arguments (the clipboard is closed again).
        Ok(unsafe { GetClipboardSequenceNumber() })
    }

    pub(super) fn restore(saved: Vec<(u32, Vec<u8>)>, our_sequence: u32) {
        // The clipboard as Utter last left it: a try that fails after emptying it changes the
        // sequence number itself, and the next try must not take that for the person's copy.
        let mut ours = our_sequence;
        let result = retrying(RESTORE_BUDGET, || {
            // Checked on every try: a copy the person makes meanwhile is never overwritten.
            // SAFETY: no arguments.
            if unsafe { GetClipboardSequenceNumber() } != ours {
                return Ok(false);
            }
            let attempt = (|| {
                let _cb = Clipboard::open()?;
                // SAFETY: the clipboard is open.
                if unsafe { EmptyClipboard() } == 0 {
                    return Err(InsertError::Clipboard(format!("empty: {}", std::io::Error::last_os_error())));
                }
                // Every format that can go back does; one that cannot (a format only its app can
                // write) is not worth losing the rest for, and trying again would not change it.
                let failed: Vec<u32> = saved.iter().filter(|(f, b)| !set_bytes(*f, b)).map(|(f, _)| *f).collect();
                if !saved.is_empty() && failed.len() == saved.len() {
                    return Err(InsertError::Clipboard("nothing could be put back".into()));
                }
                if !failed.is_empty() {
                    tracing::debug!("clipboard formats {failed:?} could not be put back");
                }
                Ok(true)
            })();
            if attempt.is_err() {
                // SAFETY: no arguments (the clipboard is closed again here).
                ours = unsafe { GetClipboardSequenceNumber() };
            }
            attempt
        });
        match result {
            Ok(true) => {}
            Ok(false) => tracing::debug!("clipboard changed since the paste; not restored"),
            Err(e) => tracing::warn!("the clipboard could not be put back: {e}"),
        }
    }

    pub(super) fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    pub(super) fn send(inputs: &[INPUT]) -> Result<(), InsertError> {
        // SAFETY: a valid slice of INPUT structs and their size.
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            return Err(InsertError::Input(format!(
                "{sent} of {} events sent: {}",
                inputs.len(),
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }

    pub fn paste(text: &str, keys: PasteKeys, restore_after: Option<Duration>) -> Result<(), InsertError> {
        let saved = save_all()?;
        let our_sequence = put_text(text)?;
        // The app reads the clipboard the moment the keys arrive, and what was just written can
        // take a moment to show (save_all): paste only once it reads back as the text. Usually
        // at once; never more than READBACK.
        let start = std::time::Instant::now();
        while clipboard_text().as_deref() != Some(text) {
            if start.elapsed() >= READBACK {
                tracing::warn!("the clipboard did not show the dictation within {READBACK:?}; pasting anyway");
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        const VK_V: u16 = 0x56;
        // Insert is an extended key: without the flag it reads as the number pad's 0.
        let chord: &[INPUT] = &match keys {
            PasteKeys::CtrlShiftV => vec![
                key(VK_CONTROL, 0, 0),
                key(VK_SHIFT, 0, 0),
                key(VK_V, 0, 0),
                key(VK_V, 0, KEYEVENTF_KEYUP),
                key(VK_SHIFT, 0, KEYEVENTF_KEYUP),
                key(VK_CONTROL, 0, KEYEVENTF_KEYUP),
            ],
            PasteKeys::ShiftInsert => vec![
                key(VK_SHIFT, 0, 0),
                key(VK_INSERT, 0, KEYEVENTF_EXTENDEDKEY),
                key(VK_INSERT, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
                key(VK_SHIFT, 0, KEYEVENTF_KEYUP),
            ],
            PasteKeys::Primary | PasteKeys::Type => vec![
                key(VK_CONTROL, 0, 0),
                key(VK_V, 0, 0),
                key(VK_V, 0, KEYEVENTF_KEYUP),
                key(VK_CONTROL, 0, KEYEVENTF_KEYUP),
            ],
        };
        send(chord)?;
        let Some(delay) = restore_after else { return Ok(()) };
        std::thread::Builder::new()
            .name("utter-clipboard".into())
            .spawn(move || {
                std::thread::sleep(delay);
                restore(saved, our_sequence);
            })
            .map_err(|e| InsertError::Clipboard(e.to_string()))?;
        Ok(())
    }

    pub fn type_text(text: &str) -> Result<(), InsertError> {
        let mut inputs = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            if unit == '\n' as u16 {
                inputs.push(key(VK_RETURN, 0, 0));
                inputs.push(key(VK_RETURN, 0, KEYEVENTF_KEYUP));
            } else if unit != '\r' as u16 {
                inputs.push(key(0, unit, KEYEVENTF_UNICODE));
                inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
            }
        }
        send(&inputs)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::time::Duration;

    use super::{DefaultRule, InsertError, PasteKeys};

    /// macOS and Linux get their tables when they are built (docs/ux/insertion.md).
    pub const DEFAULT_RULES: &[DefaultRule] = &[];

    /// Phase 4 (Linux): the RemoteDesktop portal or uinput, and wl-clipboard.
    pub fn paste(_text: &str, _keys: PasteKeys, _restore_after: Option<Duration>) -> Result<(), InsertError> {
        Err(InsertError::Input("not implemented on this platform yet".into()))
    }

    pub fn clipboard_text() -> Option<String> {
        None
    }

    pub fn put_text(_text: &str) -> Result<u32, InsertError> {
        Err(InsertError::Clipboard("not implemented on this platform yet".into()))
    }

    pub fn type_text(_text: &str) -> Result<(), InsertError> {
        Err(InsertError::Input("not implemented on this platform yet".into()))
    }
}

/// Pastes into a real window and reads back what arrived (docs/ux/insertion.md, acceptance
/// checks). It opens its own text window for a few seconds, never another app's, and puts the
/// clipboard back when done. Run by hand:
/// cargo test -p utter-platform pastes_into_a_real_window -- --ignored --nocapture
#[cfg(all(test, windows))]
mod live {
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, ES_MULTILINE, ES_WANTRETURN, GetForegroundWindow,
        MSG, PM_REMOVE, PeekMessageW, SendMessageW, SetForegroundWindow, TranslateMessage, WM_GETTEXT,
        WM_GETTEXTLENGTH, WM_SETTEXT, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    use super::{Keep, PasteKeys, PasteRule, clipboard_text, copy, imp, paste};
    use crate::focus;

    /// A plain text window of this process, gone when dropped.
    struct TestWindow(HWND);

    impl TestWindow {
        fn open(title: &str, x: i32) -> Self {
            let class: Vec<u16> = "EDIT\0".encode_utf16().collect();
            let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let style = WS_OVERLAPPEDWINDOW | WS_VISIBLE | (ES_MULTILINE | ES_WANTRETURN) as u32;
            // SAFETY: a top-level window of the system EDIT class; the strings outlive the call.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    style,
                    x,
                    120,
                    420,
                    220,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "could not create the test window");
            Self(hwnd)
        }

        /// Brings it to the front. Windows lets a background process do that right after it
        /// sends input, so a Shift tap comes first (not Alt: an Alt tap opens a window's menu
        /// mode, a modal loop that waits for the menu to close).
        fn focus(&self) {
            let shift = |flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_SHIFT, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
            };
            let taps = [shift(0), shift(KEYEVENTF_KEYUP)];
            // SAFETY: two keyboard inputs; then a window handle this process owns.
            unsafe {
                SendInput(2, taps.as_ptr(), size_of::<INPUT>() as i32);
                SetForegroundWindow(self.0);
            }
            pump(Duration::from_millis(400));
            // SAFETY: no arguments.
            assert_eq!(unsafe { GetForegroundWindow() }, self.0, "the test window did not come to the front");
        }

        fn text(&self) -> String {
            // SAFETY: WM_GETTEXT into a buffer of the size asked for, on our own window.
            unsafe {
                let n = SendMessageW(self.0, WM_GETTEXTLENGTH, 0, 0) as usize;
                let mut buf = vec![0u16; n + 1];
                let got = SendMessageW(self.0, WM_GETTEXT, buf.len() as WPARAM, buf.as_mut_ptr() as LPARAM) as usize;
                String::from_utf16_lossy(&buf[..got])
            }
        }

        fn clear(&self) {
            let empty = [0u16];
            // SAFETY: an empty NUL-terminated string to our own window.
            unsafe { SendMessageW(self.0, WM_SETTEXT, 0, empty.as_ptr() as LPARAM) };
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: a window this process created.
            unsafe { DestroyWindow(self.0) };
        }
    }

    /// Runs this thread's messages for `d`: the window needs them to take the keys.
    fn pump(d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            // SAFETY: a standard message loop on this thread's queue. Bounded by the clock too:
            // a message the window keeps regenerating must not hold it forever.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while Instant::now() < end && PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Whether the clipboard holds `text` within a second: what is written shows up a moment
    /// later, like any clipboard change (insert::imp::save_all).
    fn clipboard_becomes(text: &str) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(1) {
            if clipboard_text().as_deref() == Some(text) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn rule(keys: PasteKeys, keep: Keep) -> PasteRule {
        PasteRule { keys, keep }
    }

    #[test]
    #[ignore]
    fn the_clipboard_comes_back_whole() {
        // Clipboard managers (Windows' own history, UniClipboard, ...) open the clipboard on every
        // change to read it: each round races them, as a real paste does.
        let mine = imp::save_all().expect("read the clipboard");
        let mut lost = 0;
        for round in 0..30 {
            copy(&format!("OLD {round}")).unwrap();
            let saved = imp::save_all().unwrap();
            copy("SOMETHING ELSE").unwrap();
            // SAFETY: no arguments.
            imp::restore(saved, unsafe { GetClipboardSequenceNumber() });
            if !clipboard_becomes(&format!("OLD {round}")) {
                lost += 1;
                eprintln!("round {round}: not put back, the clipboard has {:?}", clipboard_text());
            }
        }
        // SAFETY: no arguments.
        imp::restore(mine, unsafe { GetClipboardSequenceNumber() });
        assert_eq!(lost, 0, "the clipboard did not come back {lost} times in 30");
    }

    /// How often a paste goes missing when the keys follow the clipboard write at once, and with
    /// a short settle after it: 20 pastes per delay into a real window.
    #[test]
    #[ignore]
    fn paste_misses_by_settle() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, KEYEVENTF_KEYUP as UP};
        let saved = imp::save_all().expect("read the clipboard");
        let w = TestWindow::open("Utter paste reliability", 200);
        w.clear();
        w.focus();
        for settle in [0u64, 10, 25, 50] {
            let mut missed = 0;
            for i in 0..20 {
                w.clear();
                pump(Duration::from_millis(30));
                let text = format!("paste {settle}-{i}");
                imp::put_text(&text).unwrap();
                while clipboard_text().as_deref() != Some(text.as_str()) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                std::thread::sleep(Duration::from_millis(settle));
                imp::send(&[imp::key(VK_CONTROL, 0, 0), imp::key(0x56, 0, 0), imp::key(0x56, 0, UP), imp::key(VK_CONTROL, 0, UP)])
                    .unwrap();
                pump(Duration::from_millis(120));
                if w.text() != text {
                    missed += 1;
                }
            }
            println!("settle {settle} ms: {missed} of 20 missed");
        }
        drop(w);
        // SAFETY: no arguments.
        imp::restore(saved, unsafe { GetClipboardSequenceNumber() });
    }

    #[test]
    #[ignore]
    fn pastes_into_a_real_window() {
        let saved = imp::save_all().expect("read the clipboard");
        let result = std::panic::catch_unwind(run);
        // The person's clipboard comes back, whatever happened.
        // SAFETY: no arguments.
        imp::restore(saved, unsafe { GetClipboardSequenceNumber() });
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    fn run() {
        let w = TestWindow::open("Utter paste test", 200);
        w.focus();
        let target = focus::foreground().expect("the test window has the focus");

        // Ctrl+V, Normal: the text arrives, and the old clipboard is back after 0.5 s.
        w.clear();
        copy("OLD").unwrap();
        paste(&target, "Hello from Utter.", rule(PasteKeys::Primary, Keep::Normal)).unwrap();
        assert_eq!(clipboard_text().as_deref(), Some("Hello from Utter."), "held during the paste");
        pump(Duration::from_millis(150));
        assert_eq!(w.text(), "Hello from Utter.");
        pump(Duration::from_millis(450));
        assert!(clipboard_becomes("OLD"), "restored after 0.5 s");
        println!("ctrl+v, normal: inserted, clipboard restored");

        // Shift+Insert pastes too.
        w.clear();
        paste(&target, "Shift+Insert works.", rule(PasteKeys::ShiftInsert, Keep::Normal)).unwrap();
        pump(Duration::from_millis(150));
        assert_eq!(w.text(), "Shift+Insert works.");
        println!("shift+insert: inserted");

        // Typing it out: line breaks become Enter.
        w.clear();
        pump(Duration::from_millis(700));
        paste(&target, "Typed\nout.", rule(PasteKeys::Type, Keep::Normal)).unwrap();
        pump(Duration::from_millis(200));
        assert_eq!(w.text(), "Typed\r\nout.");
        println!("type it out: inserted, line break as Enter");

        // Leave it there: the dictation stays on the clipboard.
        w.clear();
        copy("OLD").unwrap();
        paste(&target, "Left on the clipboard.", rule(PasteKeys::Primary, Keep::Leave)).unwrap();
        pump(Duration::from_millis(700));
        assert!(clipboard_becomes("Left on the clipboard."));
        println!("leave it there: not restored");

        // A copy made during the hold is never overwritten.
        w.clear();
        copy("OLD").unwrap();
        paste(&target, "Pasted.", rule(PasteKeys::Primary, Keep::Normal)).unwrap();
        pump(Duration::from_millis(100));
        copy("NEWER").unwrap();
        pump(Duration::from_millis(700));
        assert!(clipboard_becomes("NEWER"));
        println!("a newer copy: kept");

        // The focus moved: nothing is typed anywhere.
        let other = TestWindow::open("Utter paste test, other", 680);
        other.clear();
        other.focus();
        let err = paste(&target, "Must not arrive.", PasteRule::default()).unwrap_err();
        pump(Duration::from_millis(200));
        assert!(matches!(err, super::InsertError::FocusChanged(_)), "{err}");
        assert_eq!(other.text(), "");
        println!("focus moved: refused, nothing typed");
    }
}
