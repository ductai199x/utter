//! The dictation hotkey (DESIGN.md section 4.1): hold Right Ctrl to talk, double-tap it for
//! hands-free, Esc to cancel.
//!
//! Two layers:
//! - [`HotkeyMachine`]: pure logic from key events to dictation actions, unit-tested;
//! - the OS hook ([`listen`]): on Windows a low-level keyboard hook on its own thread. The
//!   hook callback only forwards the raw event and returns at once: Windows silently removes a
//!   hook whose callback is slow, and the hook never swallows a key, so every key still
//!   reaches the focused app.

use std::time::Duration;

/// A key event as the OS reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Windows virtual-key code (VK_RCONTROL is 0xA3).
    pub vk: u32,
    pub down: bool,
    /// Milliseconds on the OS's event clock.
    pub time_ms: u32,
    /// Injected by software (SendInput), not typed.
    pub injected: bool,
}

pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_ESCAPE: u32 = 0x1B;

/// What the dictation pipeline should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The key went down: start listening (the pipeline decides later whether this is a hold).
    Start,
    /// Push-to-talk ended: finish and insert.
    Finish,
    /// Hands-free began (a double tap); listening continues until [`Action::Finish`].
    HandsFree,
    /// Drop what was being recorded: a chord (Right Ctrl + another key), Esc, or a tap too
    /// short to be a dictation.
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub struct HotkeyConfig {
    pub key: u32,
    /// A press shorter than this is a tap, not a hold.
    pub min_hold: Duration,
    /// Two taps within this make a double tap.
    pub double_tap: Duration,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self { key: VK_RCONTROL, min_hold: Duration::from_millis(150), double_tap: Duration::from_millis(350) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// The key is down; `since` is when it went down.
    Held {
        since: u32,
    },
    /// A short tap just ended at `at`; a second tap soon makes it hands-free.
    Tapped {
        at: u32,
    },
    /// The second press of a double tap is down; its release leaves hands-free running.
    SecondTapDown,
    /// Hands-free: listening until the key is tapped again.
    HandsFree,
    /// The key is down to end hands-free; its release finishes.
    StopTapDown,
}

/// Key events in, actions out. Auto-repeat (down while already down) is ignored; injected
/// events (Utter's own Ctrl+V, other software) never act.
#[derive(Debug, Clone)]
pub struct HotkeyMachine {
    cfg: HotkeyConfig,
    state: State,
}

impl HotkeyMachine {
    pub fn new(cfg: HotkeyConfig) -> Self {
        Self { cfg, state: State::Idle }
    }

    pub fn on_event(&mut self, e: KeyEvent) -> Option<Action> {
        if e.injected {
            return None;
        }
        let hold_ms = self.cfg.min_hold.as_millis() as u32;
        let tap_ms = self.cfg.double_tap.as_millis() as u32;
        let is_key = e.vk == self.cfg.key;
        let listening =
            matches!(self.state, State::Held { .. } | State::SecondTapDown | State::HandsFree | State::StopTapDown);
        if listening && !is_key && e.down && e.vk == VK_ESCAPE {
            self.state = State::Idle;
            return Some(Action::Cancel);
        }
        let (next, action) = match (self.state, is_key, e.down) {
            (State::Idle, true, true) => (State::Held { since: e.time_ms }, Some(Action::Start)),
            (State::Tapped { at }, true, true) if e.time_ms.wrapping_sub(at) <= tap_ms => {
                (State::SecondTapDown, Some(Action::HandsFree))
            }
            (State::Tapped { .. }, true, true) => (State::Held { since: e.time_ms }, Some(Action::Start)),
            (State::Held { since }, true, false) if e.time_ms.wrapping_sub(since) >= hold_ms => {
                (State::Idle, Some(Action::Finish))
            }
            (State::Held { .. }, true, false) => (State::Tapped { at: e.time_ms }, Some(Action::Cancel)),
            // Another key while held: a shortcut such as Right Ctrl + C, not a dictation.
            (State::Held { .. }, false, true) => (State::Idle, Some(Action::Cancel)),
            (State::SecondTapDown, true, false) => (State::HandsFree, None),
            (State::HandsFree, true, true) => (State::StopTapDown, None),
            (State::StopTapDown, true, false) => (State::Idle, Some(Action::Finish)),
            // Auto-repeat, other keys while hands-free, releases of other keys.
            (state, _, _) => (state, None),
        };
        self.state = next;
        action
    }
}

#[derive(Debug, thiserror::Error)]
#[error("hotkey: {0}")]
pub struct HotkeyError(pub String);

/// A running hotkey listener. Dropping it removes the hook and ends its threads.
pub struct Listener {
    actions: std::sync::mpsc::Receiver<(Action, std::time::Instant)>,
    stop: Box<dyn FnMut() + Send>,
}

impl Listener {
    /// The next action, with the moment it was decided; `None` after `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<(Action, std::time::Instant)> {
        self.actions.recv_timeout(timeout).ok()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        (self.stop)();
    }
}

/// Starts listening for the hotkey system-wide.
pub fn listen(cfg: HotkeyConfig) -> Result<Listener, HotkeyError> {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<KeyEvent>();
    let (act_tx, act_rx) = std::sync::mpsc::channel();
    let stop = imp::start_hook(raw_tx)?;
    std::thread::Builder::new()
        .name("utter-hotkey".into())
        .spawn(move || {
            let mut machine = HotkeyMachine::new(cfg);
            while let Ok(e) = raw_rx.recv() {
                if let Some(a) = machine.on_event(e)
                    && act_tx.send((a, std::time::Instant::now())).is_err()
                {
                    break;
                }
            }
        })
        .map_err(|e| HotkeyError(e.to_string()))?;
    Ok(Listener { actions: act_rx, stop })
}

#[cfg(windows)]
mod imp {
    use std::sync::Mutex;
    use std::sync::mpsc::Sender;

    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, MSG, PostThreadMessageW,
        SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_QUIT,
    };

    use super::{HotkeyError, KeyEvent};

    /// Where the hook callback forwards events. One hook per process.
    static SINK: Mutex<Option<Sender<KeyEvent>>> = Mutex::new(None);

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
            let k = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let event = KeyEvent {
                vk: k.vkCode,
                down: k.flags & LLKHF_UP == 0,
                time_ms: k.time,
                injected: k.flags & LLKHF_INJECTED != 0,
            };
            // try_lock: never wait inside the hook. A missed event while the sink is being
            // replaced is better than Windows removing a slow hook.
            if let Ok(guard) = SINK.try_lock()
                && let Some(tx) = guard.as_ref()
            {
                let _ = tx.send(event);
            }
        }
        // Never swallow a key: it always reaches the focused app.
        // SAFETY: passes the hook chain on, as required.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    pub fn start_hook(tx: Sender<KeyEvent>) -> Result<Box<dyn FnMut() + Send>, HotkeyError> {
        {
            let mut sink = SINK.lock().map_err(|e| HotkeyError(e.to_string()))?;
            if sink.is_some() {
                return Err(HotkeyError("a hotkey listener is already running".into()));
            }
            *sink = Some(tx);
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new()
            .name("utter-hook".into())
            .spawn(move || {
                // SAFETY: plain Win32 calls; the hook is installed and removed on this thread,
                // which runs the message loop the hook needs.
                unsafe {
                    let module = GetModuleHandleW(std::ptr::null());
                    let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0);
                    if hook.is_null() {
                        let _ = ready_tx
                            .send(Err(format!("SetWindowsHookExW failed: {}", std::io::Error::last_os_error())));
                        return;
                    }
                    let _ = ready_tx.send(Ok(GetCurrentThreadId()));
                    let mut msg: MSG = std::mem::zeroed();
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
                    UnhookWindowsHookEx(hook);
                }
                if let Ok(mut sink) = SINK.lock() {
                    *sink = None;
                }
            })
            .map_err(|e| HotkeyError(e.to_string()))?;
        let thread_id = ready_rx.recv().map_err(|e| HotkeyError(e.to_string()))?.map_err(HotkeyError)?;
        Ok(Box::new(move || {
            // SAFETY: posts WM_QUIT to the hook thread's message queue.
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
        }))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::mpsc::Sender;

    use super::{HotkeyError, KeyEvent};

    /// Phase 4 (Linux, GNOME): the GlobalShortcuts portal or evdev (DESIGN.md section 7).
    pub fn start_hook(_tx: Sender<KeyEvent>) -> Result<Box<dyn FnMut() + Send>, HotkeyError> {
        Err(HotkeyError("not implemented on this platform yet".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(vk: u32, down: bool, t: u32) -> KeyEvent {
        KeyEvent { vk, down, time_ms: t, injected: false }
    }

    fn run(events: &[KeyEvent]) -> Vec<Action> {
        let mut m = HotkeyMachine::new(HotkeyConfig::default());
        events.iter().filter_map(|&e| m.on_event(e)).collect()
    }

    #[test]
    fn hold_then_release_finishes() {
        assert_eq!(run(&[ev(VK_RCONTROL, true, 0), ev(VK_RCONTROL, false, 900)]), vec![Action::Start, Action::Finish]);
    }

    #[test]
    fn auto_repeat_is_ignored() {
        let events = [
            ev(VK_RCONTROL, true, 0),
            ev(VK_RCONTROL, true, 500),
            ev(VK_RCONTROL, true, 530),
            ev(VK_RCONTROL, false, 900),
        ];
        assert_eq!(run(&events), vec![Action::Start, Action::Finish]);
    }

    #[test]
    fn a_short_tap_cancels() {
        assert_eq!(run(&[ev(VK_RCONTROL, true, 0), ev(VK_RCONTROL, false, 80)]), vec![Action::Start, Action::Cancel]);
    }

    #[test]
    fn a_chord_cancels_and_is_not_a_dictation() {
        // Right Ctrl + C.
        let events = [ev(VK_RCONTROL, true, 0), ev(0x43, true, 200), ev(0x43, false, 260), ev(VK_RCONTROL, false, 400)];
        assert_eq!(run(&events), vec![Action::Start, Action::Cancel]);
    }

    #[test]
    fn escape_cancels_a_hold() {
        let events = [ev(VK_RCONTROL, true, 0), ev(VK_ESCAPE, true, 500), ev(VK_RCONTROL, false, 700)];
        assert_eq!(run(&events), vec![Action::Start, Action::Cancel]);
    }

    #[test]
    fn double_tap_starts_hands_free_and_a_tap_ends_it() {
        let events = [
            ev(VK_RCONTROL, true, 0),
            ev(VK_RCONTROL, false, 90), // first tap
            ev(VK_RCONTROL, true, 250), // second tap within 350 ms
            ev(VK_RCONTROL, false, 320),
            ev(0x41, true, 2000), // typing other keys does not end hands-free
            ev(0x41, false, 2050),
            ev(VK_RCONTROL, true, 9000), // a tap ends it
            ev(VK_RCONTROL, false, 9080),
        ];
        assert_eq!(run(&events), vec![Action::Start, Action::Cancel, Action::HandsFree, Action::Finish]);
    }

    #[test]
    fn two_slow_taps_are_not_a_double_tap() {
        let events = [ev(VK_RCONTROL, true, 0), ev(VK_RCONTROL, false, 90), ev(VK_RCONTROL, true, 900)];
        assert_eq!(run(&events), vec![Action::Start, Action::Cancel, Action::Start]);
    }

    #[test]
    fn injected_events_never_act() {
        let mut e = ev(VK_RCONTROL, true, 0);
        e.injected = true;
        assert_eq!(run(&[e]), vec![]);
    }

    #[test]
    fn escape_ends_hands_free() {
        let events = [
            ev(VK_RCONTROL, true, 0),
            ev(VK_RCONTROL, false, 90),
            ev(VK_RCONTROL, true, 250),
            ev(VK_RCONTROL, false, 320),
            ev(VK_ESCAPE, true, 5000),
        ];
        assert_eq!(run(&events), vec![Action::Start, Action::Cancel, Action::HandsFree, Action::Cancel]);
    }
}
