//! The dictation hotkey (DESIGN.md section 4.1): hold the key (Right Ctrl unless the person
//! chose another) to talk, double-tap it for hands-free, Esc to cancel.
//!
//! Two layers:
//! - [`HotkeyMachine`]: pure logic from key events to dictation actions, unit-tested;
//! - the OS listener ([`listen`]): on Windows the keyboard's Raw Input on a thread of its own
//!   (see `imp` for why not a low-level hook). It only observes: every key still reaches the
//!   focused app.

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
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_SCROLL: u32 = 0x91;
/// F13; F14 to F24 follow it.
pub const VK_F13: u32 = 0x7C;
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
    /// Let injected key events act too (development builds' test dictations only: never in
    /// use, or Utter's own Ctrl+V and other software could start a dictation).
    pub accept_injected: bool,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: VK_RCONTROL,
            min_hold: Duration::from_millis(150),
            double_tap: Duration::from_millis(350),
            accept_injected: false,
        }
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
        if e.injected && !self.cfg.accept_injected {
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
    tracing::info!("hotkey: listening");
    std::thread::Builder::new()
        .name("utter-hotkey".into())
        .spawn(move || {
            let mut machine = HotkeyMachine::new(cfg);
            let mut key_down = false;
            while let Ok(e) = raw_rx.recv() {
                // The hotkey's presses and releases only (not its auto-repeat): no other key is
                // ever logged.
                if e.vk == cfg.key && e.down != key_down {
                    key_down = e.down;
                    tracing::info!(down = e.down, injected = e.injected, "hotkey: key");
                }
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

/// Windows: the keyboard's Raw Input, delivered to a message-only window on a thread of its own
/// (RIDEV_INPUTSINK: whichever window is in front).
///
/// Not a low-level keyboard hook (WH_KEYBOARD_LL): Windows silently stops calling one while a
/// Chromium window has focus, WebView2 included, so Utter's own setup and splash windows made it
/// deaf (2026-10-12; github.com/wudaming00/wh-keyboard-ll-chromium describes the same). Raw Input
/// has no hook chain and no timeout to fall out of. It only observes: every key still reaches
/// the focused app.
///
/// Raw Input is one target per process and device type. The window library (tao) registers the
/// keyboard for its own window at startup and never again; this registration replaces it, which
/// costs Utter nothing (no window uses tao's keyboard device events), and its mouse registration
/// stays.
#[cfg(windows)]
mod imp {
    use std::sync::Mutex;
    use std::sync::mpsc::Sender;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LCONTROL, VK_LSHIFT, VK_RCONTROL, VK_RSHIFT, VK_SHIFT,
    };
    use windows_sys::Win32::UI::Input::{
        GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE,
        RIM_TYPEKEYBOARD, RegisterRawInputDevices,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageTime, GetMessageW, HWND_MESSAGE,
        MSG, PostThreadMessageW, RI_KEY_BREAK, RI_KEY_E0, RegisterClassW, WM_INPUT, WM_QUIT, WNDCLASSW,
    };

    use super::{HotkeyError, KeyEvent};

    /// Where the window forwards events. One listener per process.
    static SINK: Mutex<Option<Sender<KeyEvent>>> = Mutex::new(None);

    /// HID usage page and usage of a keyboard.
    const GENERIC_DESKTOP: u16 = 0x01;
    const KEYBOARD: u16 = 0x06;
    /// The prefix of Pause's fake Ctrl, which is not a Ctrl press.
    const RI_KEY_E1: u32 = 4;
    /// Right Shift's scan code (Left Shift's is 0x2A).
    const RIGHT_SHIFT_SCAN: u16 = 0x36;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The key event in a WM_INPUT, if it is the keyboard's.
    fn key_event(raw: HRAWINPUT) -> Option<KeyEvent> {
        let mut input: RAWINPUT = RAWINPUT::default();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        // SAFETY: reads this WM_INPUT's data into a RAWINPUT, which holds any keyboard input.
        let read = unsafe {
            GetRawInputData(
                raw,
                RID_INPUT,
                (&raw mut input).cast(),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if read == u32::MAX || input.header.dwType != RIM_TYPEKEYBOARD {
            return None;
        }
        // SAFETY: the header says the data is the keyboard's.
        let k = unsafe { input.data.keyboard };
        let flags = u32::from(k.Flags);
        if flags & RI_KEY_E1 != 0 || k.VKey == 0xFF {
            return None;
        }
        // Raw Input reports either Ctrl as VK_CONTROL, the E0 prefix marking the right one, and
        // either Shift as VK_SHIFT, told apart by scan code. A Shift with the E0 prefix is one
        // the keyboard fakes around other keys (arrows with Num Lock on): no key was pressed.
        let vk = match k.VKey {
            VK_CONTROL if flags & RI_KEY_E0 != 0 => VK_RCONTROL,
            VK_CONTROL => VK_LCONTROL,
            VK_SHIFT if flags & RI_KEY_E0 != 0 => return None,
            VK_SHIFT if k.MakeCode == RIGHT_SHIFT_SCAN => VK_RSHIFT,
            VK_SHIFT => VK_LSHIFT,
            vk => vk,
        };
        Some(KeyEvent {
            vk: u32::from(vk),
            down: flags & RI_KEY_BREAK == 0,
            // SAFETY: the time of the message being handled, the clock key events use.
            time_ms: unsafe { GetMessageTime() } as u32,
            // Input from software (SendInput: Utter's own Ctrl+V, other programs) comes from no
            // device.
            injected: input.header.hDevice.is_null(),
        })
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_INPUT
            && let Some(event) = key_event(lparam as HRAWINPUT)
            && let Ok(guard) = SINK.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(event);
        }
        // SAFETY: the default handling (for WM_INPUT, it frees the input's data).
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn register(hwnd: HWND, flags: u32) -> bool {
        let device =
            RAWINPUTDEVICE { usUsagePage: GENERIC_DESKTOP, usUsage: KEYBOARD, dwFlags: flags, hwndTarget: hwnd };
        // SAFETY: one device description, of its own size.
        unsafe { RegisterRawInputDevices(&device, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0 }
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
        let thread = std::thread::Builder::new()
            .name("utter-keys-input".into())
            .spawn(move || {
                let fail = |what: &str| {
                    let error = std::io::Error::last_os_error();
                    if let Ok(mut sink) = SINK.lock() {
                        *sink = None;
                    }
                    let _ = ready_tx.send(Err(format!("{what} failed: {error}")));
                };
                let class = wide("UtterHotkeyInput");
                // SAFETY: plain Win32 calls; the window is created, receives input and is
                // destroyed on this thread, which runs its message loop.
                unsafe {
                    let module = GetModuleHandleW(std::ptr::null());
                    let wc = WNDCLASSW {
                        lpfnWndProc: Some(window_proc),
                        hInstance: module,
                        lpszClassName: class.as_ptr(),
                        ..std::mem::zeroed()
                    };
                    // A second listener (Retry) finds the class registered already: fine.
                    RegisterClassW(&wc);
                    let hwnd = CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        HWND_MESSAGE,
                        std::ptr::null_mut(),
                        module,
                        std::ptr::null(),
                    );
                    if hwnd.is_null() {
                        return fail("CreateWindowExW");
                    }
                    if !register(hwnd, RIDEV_INPUTSINK) {
                        DestroyWindow(hwnd);
                        return fail("RegisterRawInputDevices");
                    }
                    let _ = ready_tx.send(Ok(GetCurrentThreadId()));
                    let mut msg: MSG = std::mem::zeroed();
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        DispatchMessageW(&msg);
                    }
                    register(std::ptr::null_mut(), RIDEV_REMOVE);
                    DestroyWindow(hwnd);
                }
                if let Ok(mut sink) = SINK.lock() {
                    *sink = None;
                }
            })
            .map_err(|e| HotkeyError(e.to_string()))?;
        let thread_id = ready_rx.recv().map_err(|e| HotkeyError(e.to_string()))?.map_err(HotkeyError)?;
        let mut thread = Some(thread);
        Ok(Box::new(move || {
            // SAFETY: posts WM_QUIT to the listener thread's message queue.
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
            // Waits until the window is gone and the sink cleared, so a new listener can start
            // right after.
            if let Some(t) = thread.take() {
                let _ = t.join();
            }
        }))
    }
}

/// Linux: every keyboard through the kernel (linux_input.rs), under X11 and Wayland alike. The
/// GlobalShortcuts portal cannot bind a lone modifier such as Right Ctrl, nor report a key
/// held, so it is not used.
#[cfg(target_os = "linux")]
mod imp {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    use super::{HotkeyError, KeyEvent};

    pub fn start_hook(tx: Sender<KeyEvent>) -> Result<Box<dyn FnMut() + Send>, HotkeyError> {
        let rx = crate::linux_input::subscribe().map_err(HotkeyError)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::Builder::new()
            .name("utter-keys-input".into())
            .spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(e) => {
                            if tx.send(e).is_err() {
                                break;
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| HotkeyError(e.to_string()))?;
        Ok(Box::new(move || stop.store(true, Ordering::Relaxed)))
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
mod imp {
    use std::sync::mpsc::Sender;

    use super::{HotkeyError, KeyEvent};

    pub fn start_hook(tx: Sender<KeyEvent>) -> Result<Box<dyn FnMut() + Send>, HotkeyError> {
        crate::macos::start_hook(tx).map_err(HotkeyError)
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
