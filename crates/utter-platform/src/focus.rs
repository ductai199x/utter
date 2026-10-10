//! The window that has the keyboard focus: captured when the hotkey goes down, so the text
//! goes to that window and nowhere else (DESIGN.md section 4, step 2).

/// A top-level window, as captured at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The OS handle (an HWND on Windows), as an integer.
    pub handle: isize,
    pub process_id: u32,
    pub title: String,
}

/// The foreground window now, if any.
pub fn foreground() -> Option<Window> {
    imp::foreground()
}

/// Whether `w` is still the foreground window.
pub fn is_foreground(w: &Window) -> bool {
    imp::foreground_handle() == Some(w.handle)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

    use super::Window;

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
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            Some(Window { handle: hwnd as isize, process_id: pid, title: String::from_utf16_lossy(&buf[..n]) })
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Window;

    pub fn foreground_handle() -> Option<isize> {
        None
    }

    /// Phase 4 (Linux): AT-SPI and the portal (DESIGN.md section 7).
    pub fn foreground() -> Option<Window> {
        None
    }
}
