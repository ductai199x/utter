//! The Bar's window rules (docs/ux/bar.md, "Focus and input rules"): it never takes the
//! focus, has no taskbar button, and appears at the bottom center of the monitor holding the
//! focused window, without activating.
//!
//! Click-through is set by the UI toolkit (Tauri's `set_ignore_cursor_events`), which manages
//! the layered-window style it needs.

/// Gives an existing top-level window the overlay styles: never activated, no taskbar button.
pub fn make_overlay(handle: isize) {
    imp::make_overlay(handle);
}

/// Places the window at the bottom center of the work area (above the taskbar) of the monitor
/// that holds `near` (a window handle) or, without one, the mouse; `size` and `bottom_margin`
/// are in logical pixels, scaled by that monitor's DPI. Shows it without activating it.
pub fn show_bottom_center(handle: isize, near: Option<isize>, size: (u32, u32), bottom_margin: u32) {
    imp::show_bottom_center(handle, near, size, bottom_margin);
}

pub fn hide(handle: isize) {
    imp::hide(handle);
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetCursorPos, GetWindowLongPtrW, HWND_TOPMOST, SW_HIDE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
        SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindowAsync, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    pub fn make_overlay(handle: isize) {
        let hwnd = handle as HWND;
        // SAFETY: reads and sets extended styles of a window this process owns.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize);
        }
    }

    pub fn show_bottom_center(handle: isize, near: Option<isize>, size: (u32, u32), bottom_margin: u32) {
        // SAFETY: plain Win32 queries and one SetWindowPos on a window this process owns.
        unsafe {
            let monitor = match near {
                Some(w) => MonitorFromWindow(w as HWND, MONITOR_DEFAULTTONEAREST),
                None => {
                    let mut p = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut p);
                    MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
                }
            };
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return;
            }
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let scale = |v: u32| (v as f64 * dpi_x as f64 / 96.0).round() as i32;
            let (w, h) = (scale(size.0), scale(size.1));
            let work = info.rcWork;
            let x = work.left + (work.right - work.left - w) / 2;
            let y = work.bottom - h - scale(bottom_margin);
            // Async: the window belongs to the UI thread, and the caller (dictation) must not wait
            // for it.
            SetWindowPos(
                handle as HWND,
                HWND_TOPMOST,
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_ASYNCWINDOWPOS,
            );
        }
    }

    pub fn hide(handle: isize) {
        // SAFETY: hides a window this process owns, without waiting for its thread.
        unsafe { ShowWindowAsync(handle as HWND, SW_HIDE) };
    }
}

#[cfg(not(windows))]
mod imp {
    /// Phase 4 (Linux): a layer-shell or always-on-top window per compositor.
    pub fn make_overlay(_handle: isize) {}
    pub fn show_bottom_center(_handle: isize, _near: Option<isize>, _size: (u32, u32), _bottom_margin: u32) {}
    pub fn hide(_handle: isize) {}
}
