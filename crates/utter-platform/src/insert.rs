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
        /// Opens the clipboard, retrying briefly: another app may hold it for a moment.
        fn open() -> Result<Self, InsertError> {
            for _ in 0..20 {
                // SAFETY: null owner window; paired with CloseClipboard in Drop.
                if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                    return Ok(Clipboard);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(InsertError::Clipboard(format!("busy: {}", std::io::Error::last_os_error())))
        }
    }

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

    fn save_all() -> Result<Vec<(u32, Vec<u8>)>, InsertError> {
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
                continue;
            }
            // SAFETY: every format not in HANDLE_FORMATS holds global memory.
            if let Some(bytes) = unsafe { read_global(handle) } {
                saved.push((format, bytes));
            }
        }
        Ok(saved)
    }

    pub fn put_text(text: &str) -> Result<u32, InsertError> {
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
        // SAFETY: no arguments.
        Ok(unsafe { GetClipboardSequenceNumber() })
    }

    fn restore(saved: Vec<(u32, Vec<u8>)>, our_sequence: u32) {
        // SAFETY: no arguments.
        if unsafe { GetClipboardSequenceNumber() } != our_sequence {
            tracing::debug!("clipboard changed since the paste; not restored");
            return;
        }
        let Ok(_cb) = Clipboard::open() else { return };
        // SAFETY: the clipboard is open.
        unsafe { EmptyClipboard() };
        for (format, bytes) in saved {
            set_bytes(format, &bytes);
        }
    }

    fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn send(inputs: &[INPUT]) -> Result<(), InsertError> {
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

    pub fn put_text(_text: &str) -> Result<u32, InsertError> {
        Err(InsertError::Clipboard("not implemented on this platform yet".into()))
    }

    pub fn type_text(_text: &str) -> Result<(), InsertError> {
        Err(InsertError::Input("not implemented on this platform yet".into()))
    }
}
