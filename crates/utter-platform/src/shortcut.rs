//! System-wide shortcuts besides the dictation key (docs/ux/insertion.md): paste and copy the
//! last dictation. Registered with the OS (RegisterHotKey on Windows), so they reach Utter in
//! every app and the app itself never sees them. A shortcut another program holds is reported,
//! not fatal: the tray items still work.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Alt+Shift+Z (Ctrl+Cmd+V on macOS).
    PasteLast,
    /// Alt+Shift+X (Ctrl+Cmd+C on macOS).
    CopyLast,
}

impl Shortcut {
    pub const ALL: [Shortcut; 2] = [Shortcut::PasteLast, Shortcut::CopyLast];

    /// The keys, as this platform writes them.
    pub fn keys(self) -> &'static str {
        imp::keys(self)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("shortcuts: {0}")]
pub struct ShortcutError(pub String);

/// The registered shortcuts; dropping it unregisters them.
pub struct Shortcuts {
    stop: Box<dyn FnMut() + Send>,
    /// The ones another program already holds.
    pub taken: Vec<Shortcut>,
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        (self.stop)();
    }
}

/// Registers every shortcut; `on` runs, on the shortcuts' own thread, for each press.
pub fn register(on: impl FnMut(Shortcut) + Send + 'static) -> Result<Shortcuts, ShortcutError> {
    imp::register(Box::new(on))
}

/// Waits until no modifier key (Ctrl, Alt, Shift, the Windows key) is held, up to `timeout`;
/// `false` if one still is. A shortcut fires while its keys are down, and paste keys sent then
/// would reach the app with them (Ctrl+Alt+Shift+V).
pub fn wait_for_modifiers_released(timeout: Duration) -> bool {
    let start = Instant::now();
    while imp::modifiers_down() {
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MOD_ALT, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_CONTROL, VK_LWIN,
        VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PostThreadMessageW, WM_HOTKEY, WM_QUIT};

    use super::{Shortcut, ShortcutError, Shortcuts};

    pub fn keys(s: Shortcut) -> &'static str {
        match s {
            Shortcut::PasteLast => "Alt+Shift+Z",
            Shortcut::CopyLast => "Alt+Shift+X",
        }
    }

    fn vk(s: Shortcut) -> u32 {
        match s {
            Shortcut::PasteLast => 'Z' as u32,
            Shortcut::CopyLast => 'X' as u32,
        }
    }

    pub fn register(mut on: Box<dyn FnMut(Shortcut) + Send>) -> Result<Shortcuts, ShortcutError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<(u32, Vec<Shortcut>)>();
        std::thread::Builder::new()
            .name("utter-shortcuts".into())
            .spawn(move || {
                // SAFETY: plain Win32 calls; the hot keys are registered and unregistered on this
                // thread, whose message queue receives their WM_HOTKEY messages.
                unsafe {
                    let mut taken = Vec::new();
                    for (id, s) in Shortcut::ALL.into_iter().enumerate() {
                        if RegisterHotKey(
                            std::ptr::null_mut(),
                            id as i32 + 1,
                            MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                            vk(s),
                        ) == 0
                        {
                            taken.push(s);
                        }
                    }
                    let _ = ready_tx.send((GetCurrentThreadId(), taken));
                    let mut msg: MSG = std::mem::zeroed();
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        if msg.message == WM_HOTKEY
                            && let Some(&s) = Shortcut::ALL.get((msg.wParam as usize).wrapping_sub(1))
                        {
                            on(s);
                        }
                    }
                    for id in 1..=Shortcut::ALL.len() {
                        UnregisterHotKey(std::ptr::null_mut(), id as i32);
                    }
                }
            })
            .map_err(|e| ShortcutError(e.to_string()))?;
        let (thread_id, taken) = ready_rx.recv().map_err(|e| ShortcutError(e.to_string()))?;
        Ok(Shortcuts {
            stop: Box::new(move || {
                // SAFETY: posts WM_QUIT to the shortcut thread's message queue.
                unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
            }),
            taken,
        })
    }

    pub fn modifiers_down() -> bool {
        // SAFETY: plain queries of the physical key state; the high bit means "down now".
        [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].iter().any(|&k| unsafe { GetAsyncKeyState(k as i32) } < 0)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Shortcut, ShortcutError, Shortcuts};

    pub fn keys(s: Shortcut) -> &'static str {
        match s {
            Shortcut::PasteLast => "Alt+Shift+Z",
            Shortcut::CopyLast => "Alt+Shift+X",
        }
    }

    /// Phase 4 (Linux, GNOME): the GlobalShortcuts portal (DESIGN.md section 7).
    pub fn register(_on: Box<dyn FnMut(Shortcut) + Send>) -> Result<Shortcuts, ShortcutError> {
        Err(ShortcutError("not implemented on this platform yet".into()))
    }

    pub fn modifiers_down() -> bool {
        false
    }
}
