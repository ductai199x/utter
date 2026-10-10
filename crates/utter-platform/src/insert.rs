//! Putting text into the focused app (DESIGN.md section 4, step 8).
//!
//! [`paste`]: save the clipboard, put the text on it, send Ctrl+V, and restore the clipboard
//! once the app has had time to read it. Rules:
//! - nothing is typed unless the window captured at key down still has the focus;
//! - every clipboard format held in global memory is saved and put back (formats that are GDI
//!   handles, such as CF_BITMAP, cannot be copied; their memory-based equivalents, such as
//!   CF_DIB, are kept);
//! - the clipboard is restored only if nothing else has changed it since the paste, so a copy
//!   the user makes in the meantime is never overwritten;
//! - the dictated text is marked to stay out of Windows' clipboard history and cloud clipboard.
//!
//! [`type_text`] is the fallback: the text as Unicode key strokes, for apps that do not paste.

use std::time::Duration;

use crate::focus::{self, Window};

#[derive(Debug, thiserror::Error)]
pub enum InsertError {
    #[error("the focus moved away from \"{0}\"; nothing was typed")]
    FocusChanged(String),
    #[error("clipboard: {0}")]
    Clipboard(String),
    #[error("sending keys: {0}")]
    Input(String),
}

#[derive(Debug, Clone, Copy)]
pub struct PasteOptions {
    /// How long the app gets to read the clipboard before it is restored.
    pub restore_after: Duration,
}

impl Default for PasteOptions {
    fn default() -> Self {
        Self { restore_after: Duration::from_millis(250) }
    }
}

/// Pastes `text` into `target`. Returns once the paste is sent; the clipboard is restored on a
/// background thread after `opts.restore_after`.
pub fn paste(target: &Window, text: &str, opts: PasteOptions) -> Result<(), InsertError> {
    if !focus::is_foreground(target) {
        return Err(InsertError::FocusChanged(target.title.clone()));
    }
    imp::paste(text, opts)
}

/// Types `text` as Unicode key strokes into `target` (line breaks as Enter).
pub fn type_text(target: &Window, text: &str) -> Result<(), InsertError> {
    if !focus::is_foreground(target) {
        return Err(InsertError::FocusChanged(target.title.clone()));
    }
    imp::type_text(text)
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
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, VK_CONTROL,
        VK_RETURN,
    };

    use super::{InsertError, PasteOptions};

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

    fn put_text(text: &str) -> Result<u32, InsertError> {
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

    pub fn paste(text: &str, opts: PasteOptions) -> Result<(), InsertError> {
        let saved = save_all()?;
        let our_sequence = put_text(text)?;
        const VK_V: u16 = 0x56;
        send(&[
            key(VK_CONTROL, 0, 0),
            key(VK_V, 0, 0),
            key(VK_V, 0, KEYEVENTF_KEYUP),
            key(VK_CONTROL, 0, KEYEVENTF_KEYUP),
        ])?;
        let delay = opts.restore_after;
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
    use super::{InsertError, PasteOptions};

    /// Phase 4 (Linux): the RemoteDesktop portal or uinput, and wl-clipboard.
    pub fn paste(_text: &str, _opts: PasteOptions) -> Result<(), InsertError> {
        Err(InsertError::Input("not implemented on this platform yet".into()))
    }

    pub fn type_text(_text: &str) -> Result<(), InsertError> {
        Err(InsertError::Input("not implemented on this platform yet".into()))
    }
}
