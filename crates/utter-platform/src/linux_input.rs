//! The keyboard on Linux, under X11 and Wayland alike, through the kernel (evdev):
//! - every keyboard read on one thread and each key event handed to every subscriber (the
//!   dictation hotkey, the paste-last shortcuts), keyboards plugged in later included; Wayland
//!   gives a normal app no other way to see a key held over another window;
//! - a virtual keyboard (uinput) for the paste keys, which reaches X11 and Wayland apps alike.
//!
//! Reading `/dev/input/event*` and writing `/dev/uinput` take a udev rule giving the person at
//! the seat access (`uaccess`), which Utter's package installs (packaging/linux).

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use evdev::{Device, EventSummary, KeyCode};

use crate::hotkey::KeyEvent;

/// The name of Utter's own virtual keyboard: its key events are marked injected, so its Ctrl+V
/// never acts as the hotkey.
pub const VIRTUAL_KEYBOARD: &str = "Utter virtual keyboard";

/// How often the device list is looked at again, for keyboards plugged in later.
const RESCAN: Duration = Duration::from_secs(2);

/// Linux key codes as the hotkey machine's Windows virtual-key codes (hotkey.rs speaks VK):
/// the keys the hotkey can be, Esc, and the modifiers; any other key keeps its code above
/// 0x10000, which is never a hotkey but still counts as "another key".
pub fn vk(code: KeyCode) -> u32 {
    match code {
        KeyCode::KEY_RIGHTCTRL => 0xA3,
        KeyCode::KEY_LEFTCTRL => 0xA2,
        KeyCode::KEY_RIGHTSHIFT => 0xA1,
        KeyCode::KEY_LEFTSHIFT => 0xA0,
        KeyCode::KEY_RIGHTALT => 0xA5,
        KeyCode::KEY_LEFTALT => 0xA4,
        KeyCode::KEY_LEFTMETA => 0x5B,
        KeyCode::KEY_RIGHTMETA => 0x5C,
        KeyCode::KEY_SCROLLLOCK => 0x91,
        KeyCode::KEY_ESC => 0x1B,
        KeyCode::KEY_F13 => 0x7C,
        KeyCode::KEY_F14 => 0x7D,
        KeyCode::KEY_F15 => 0x7E,
        KeyCode::KEY_F16 => 0x7F,
        KeyCode::KEY_F17 => 0x80,
        KeyCode::KEY_F18 => 0x81,
        KeyCode::KEY_F19 => 0x82,
        KeyCode::KEY_F20 => 0x83,
        KeyCode::KEY_F21 => 0x84,
        KeyCode::KEY_F22 => 0x85,
        KeyCode::KEY_F23 => 0x86,
        KeyCode::KEY_F24 => 0x87,
        KeyCode::KEY_Z => 0x5A,
        KeyCode::KEY_X => 0x58,
        other => 0x1_0000 + u32::from(other.code()),
    }
}

/// Modifier keys held now (any keyboard), as bits: Ctrl, Shift, Alt, Meta.
static MODIFIERS: AtomicU8 = AtomicU8::new(0);
pub const CTRL: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const META: u8 = 8;

pub fn modifiers() -> u8 {
    MODIFIERS.load(Ordering::Relaxed)
}

fn modifier_bit(code: KeyCode) -> u8 {
    match code {
        KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => CTRL,
        KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => SHIFT,
        KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => ALT,
        KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => META,
        _ => 0,
    }
}

struct Hub {
    subscribers: Vec<Sender<KeyEvent>>,
}

fn hub() -> &'static Mutex<Hub> {
    static HUB: OnceLock<Mutex<Hub>> = OnceLock::new();
    HUB.get_or_init(|| Mutex::new(Hub { subscribers: Vec::new() }))
}

/// Key events from every keyboard, from now on. Fails, saying why, when no keyboard can be
/// read (the udev rule is missing, or the person is not at the seat).
pub fn subscribe() -> Result<Receiver<KeyEvent>, String> {
    // Started once, by the first call that finds a keyboard: a failed start is tried again.
    static READER: Mutex<bool> = Mutex::new(false);
    {
        let mut started = READER.lock().unwrap_or_else(|e| e.into_inner());
        if !*started {
            start_reader()?;
            *started = true;
        }
    }
    let (tx, rx) = mpsc::channel();
    hub().lock().expect("input hub").subscribers.push(tx);
    Ok(rx)
}

/// The keyboards among the input devices, readable by this user, with Utter's own last.
fn keyboards(known: &HashMap<PathBuf, Device>) -> (Vec<(PathBuf, Device)>, usize) {
    let mut found = Vec::new();
    let mut denied = 0;
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return (found, 0) };
    for entry in dir.flatten() {
        let path = entry.path();
        let is_event = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event"));
        if !is_event || known.contains_key(&path) {
            continue;
        }
        match Device::open(&path) {
            Ok(dev) => {
                let keyboard = dev.supported_keys().is_some_and(|k| {
                    k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_Z) && k.contains(KeyCode::KEY_ENTER)
                });
                if keyboard {
                    found.push((path, dev));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied += 1,
            Err(_) => {}
        }
    }
    (found, denied)
}

const NO_ACCESS: &str =
    "the keyboard devices are not open to this user (Utter's udev rule, packaging/linux/70-utter.rules)";

fn start_reader() -> Result<(), String> {
    let (first, denied) = keyboards(&HashMap::new());
    if first.is_empty() {
        return Err(if denied > 0 { NO_ACCESS.into() } else { "no keyboard found".into() });
    }
    std::thread::Builder::new()
        .name("utter-keyboards".into())
        .spawn(move || read_loop(first))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Takes a keyboard into the read loop: nonblocking (one poll over every device, then each read
/// drains what is there), and marked injected when it is Utter's own.
fn add(devices: &mut HashMap<PathBuf, Device>, injected: &mut HashMap<PathBuf, bool>, path: PathBuf, dev: Device) {
    // SAFETY: setting O_NONBLOCK on a descriptor the device owns.
    unsafe {
        let fd = dev.as_raw_fd();
        libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK);
    }
    let ours = dev.name().is_some_and(|n| n == VIRTUAL_KEYBOARD);
    tracing::info!(device = %path.display(), name = dev.name().unwrap_or("?"), ours, "keyboard");
    injected.insert(path.clone(), ours);
    devices.insert(path, dev);
}

fn read_loop(first: Vec<(PathBuf, Device)>) {
    let mut devices: HashMap<PathBuf, Device> = HashMap::new();
    let mut injected: HashMap<PathBuf, bool> = HashMap::new();
    for (p, d) in first {
        add(&mut devices, &mut injected, p, d);
    }
    let mut scanned = Instant::now();
    loop {
        if scanned.elapsed() >= RESCAN {
            scanned = Instant::now();
            let (new, _) = keyboards(&devices);
            for (p, d) in new {
                add(&mut devices, &mut injected, p, d);
            }
        }
        let paths: Vec<PathBuf> = devices.keys().cloned().collect();
        let mut fds: Vec<libc::pollfd> = paths
            .iter()
            .map(|p| libc::pollfd { fd: devices[p].as_raw_fd(), events: libc::POLLIN, revents: 0 })
            .collect();
        // SAFETY: a valid array of pollfd for the call.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 500) };
        if n <= 0 {
            if devices.is_empty() {
                std::thread::sleep(RESCAN);
            }
            continue;
        }
        let mut gone = Vec::new();
        for (i, path) in paths.iter().enumerate() {
            let revents = fds[i].revents;
            if revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                gone.push(path.clone());
                continue;
            }
            if revents & libc::POLLIN == 0 {
                continue;
            }
            let ours = injected.get(path).copied().unwrap_or(false);
            let Some(dev) = devices.get_mut(path) else { continue };
            let events: Vec<_> = match dev.fetch_events() {
                Ok(events) => events.collect(),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => {
                    gone.push(path.clone());
                    continue;
                }
            };
            for ev in events {
                if let EventSummary::Key(_, code, value) = ev.destructure() {
                    let down = value != 0; // 1 press, 2 auto-repeat, 0 release
                    let bit = modifier_bit(code);
                    if bit != 0 && !ours {
                        if down {
                            MODIFIERS.fetch_or(bit, Ordering::Relaxed);
                        } else {
                            MODIFIERS.fetch_and(!bit, Ordering::Relaxed);
                        }
                    }
                    let time_ms =
                        ev.timestamp().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u32).unwrap_or_default();
                    let event = KeyEvent { vk: vk(code), down, time_ms, injected: ours };
                    hub().lock().expect("input hub").subscribers.retain(|s| s.send(event).is_ok());
                }
            }
        }
        for path in gone {
            tracing::info!(device = %path.display(), "keyboard gone");
            devices.remove(&path);
            injected.remove(&path);
        }
    }
}

/// Utter's virtual keyboard and when the compositor will have taken it up (about a second
/// after it is made).
struct Virtual {
    dev: evdev::uinput::VirtualDevice,
    ready: Instant,
}

/// The virtual keyboard, made once and kept: devices made and dropped per keystroke crashed
/// GNOME Shell on Xorg (ydotool, 2026-10-13). Not made yet while /dev/uinput is closed to this
/// user: the next use tries again, so allowing it (allow_input) works without a restart.
static KEYBOARD: Mutex<Option<Virtual>> = Mutex::new(None);

fn with_keyboard<T>(f: impl FnOnce(&mut Virtual) -> Result<T, String>) -> Result<T, String> {
    let mut slot = KEYBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let kb = match slot.as_mut() {
        Some(kb) => kb,
        None => {
            // Every key: a device with fewer is not taken for a keyboard by udev and libinput.
            let mut keys = evdev::AttributeSet::<KeyCode>::new();
            for code in 1..249u16 {
                keys.insert(KeyCode::new(code));
            }
            let dev = evdev::uinput::VirtualDevice::builder()
                .map_err(|e| format!("/dev/uinput: {e}"))?
                .name(VIRTUAL_KEYBOARD)
                .with_keys(&keys)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| format!("/dev/uinput: {e}"))?;
            slot.insert(Virtual { dev, ready: Instant::now() + Duration::from_millis(1200) })
        }
    };
    f(kb)
}

/// Makes the virtual keyboard now, so the first paste does not wait for the compositor to take
/// it up.
pub fn warm() {
    std::thread::spawn(|| {
        if let Err(e) = with_keyboard(|_| Ok(())) {
            tracing::warn!("virtual keyboard: {e}");
        }
    });
}

/// Presses `combo` (modifiers first, released in reverse) on the virtual keyboard.
pub fn press(combo: &[KeyCode]) -> Result<(), String> {
    with_keyboard(|kb| {
        if let Some(wait) = kb.ready.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let key = |code: KeyCode, value: i32| evdev::InputEvent::new(evdev::EventType::KEY.0, code.code(), value);
        for &c in combo {
            kb.dev.emit(&[key(c, 1)]).map_err(|e| e.to_string())?;
        }
        std::thread::sleep(Duration::from_millis(8));
        for &c in combo.iter().rev() {
            kb.dev.emit(&[key(c, 0)]).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

/// Whether this user may read the keyboards and make the virtual one, and if not, why.
pub fn access() -> Result<(), String> {
    let (found, denied) = keyboards(&HashMap::new());
    if found.is_empty() {
        return Err(if denied > 0 { NO_ACCESS.into() } else { "no keyboard found".into() });
    }
    std::fs::OpenOptions::new().write(true).open("/dev/uinput").map_err(|e| format!("/dev/uinput: {e}"))?;
    Ok(())
}

/// The udev rule that gives the person at the seat the keyboards and /dev/uinput.
pub const UDEV_RULE: &str = include_str!("../../../packaging/linux/70-utter.rules");

/// Installs the udev rule (packaging/linux), through pkexec: the desktop asks for the person's
/// password in its own dialog. The package managers' installs do this themselves; this is for
/// the AppImage, and for a rule since removed.
pub fn allow_input() -> Result<(), String> {
    use std::io::Write;
    // Loads uinput (a static node otherwise, made accessible at boot), puts the rule in place
    // and applies it to the devices there now, so it works without logging out.
    const SCRIPT: &str = "modprobe uinput 2>/dev/null; \
        cat > /etc/udev/rules.d/70-utter.rules && \
        udevadm control --reload-rules && \
        udevadm trigger --action=change --subsystem-match=input --subsystem-match=misc && \
        udevadm settle --timeout=5";
    let mut child = std::process::Command::new("pkexec")
        .args(["/bin/sh", "-c", SCRIPT])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("pkexec: {e}"))?;
    child.stdin.take().expect("piped stdin").write_all(UDEV_RULE.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => access(),
        // pkexec: the dialog was dismissed (126) or no agent could ask (127).
        Some(126) => Err("cancelled".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
    }
}

/// The US-layout keys that type `ch`, if it is plain ASCII.
pub fn keys_for(ch: char) -> Option<Vec<KeyCode>> {
    const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";
    const PLAIN: &str = "`1234567890-=[]\\;',./";
    let (shift, base) = match SHIFTED.find(ch) {
        Some(i) => (true, PLAIN.as_bytes()[i] as char),
        None if ch.is_ascii_uppercase() => (true, ch.to_ascii_lowercase()),
        None => (false, ch),
    };
    let code = match base {
        'a'..='z' => {
            const LETTERS: [KeyCode; 26] = [
                KeyCode::KEY_A,
                KeyCode::KEY_B,
                KeyCode::KEY_C,
                KeyCode::KEY_D,
                KeyCode::KEY_E,
                KeyCode::KEY_F,
                KeyCode::KEY_G,
                KeyCode::KEY_H,
                KeyCode::KEY_I,
                KeyCode::KEY_J,
                KeyCode::KEY_K,
                KeyCode::KEY_L,
                KeyCode::KEY_M,
                KeyCode::KEY_N,
                KeyCode::KEY_O,
                KeyCode::KEY_P,
                KeyCode::KEY_Q,
                KeyCode::KEY_R,
                KeyCode::KEY_S,
                KeyCode::KEY_T,
                KeyCode::KEY_U,
                KeyCode::KEY_V,
                KeyCode::KEY_W,
                KeyCode::KEY_X,
                KeyCode::KEY_Y,
                KeyCode::KEY_Z,
            ];
            LETTERS[(base as u8 - b'a') as usize]
        }
        '1'..='9' => KeyCode::new(KeyCode::KEY_1.code() + (base as u16 - '1' as u16)),
        '0' => KeyCode::KEY_0,
        ' ' => KeyCode::KEY_SPACE,
        '\n' => KeyCode::KEY_ENTER,
        '\t' => KeyCode::KEY_TAB,
        '-' => KeyCode::KEY_MINUS,
        '=' => KeyCode::KEY_EQUAL,
        '[' => KeyCode::KEY_LEFTBRACE,
        ']' => KeyCode::KEY_RIGHTBRACE,
        '\\' => KeyCode::KEY_BACKSLASH,
        ';' => KeyCode::KEY_SEMICOLON,
        '\'' => KeyCode::KEY_APOSTROPHE,
        '`' => KeyCode::KEY_GRAVE,
        ',' => KeyCode::KEY_COMMA,
        '.' => KeyCode::KEY_DOT,
        '/' => KeyCode::KEY_SLASH,
        _ => return None,
    };
    Some(if shift { vec![KeyCode::KEY_LEFTSHIFT, code] } else { vec![code] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_candidates_map_to_their_windows_codes() {
        assert_eq!(vk(KeyCode::KEY_RIGHTCTRL), crate::hotkey::VK_RCONTROL);
        assert_eq!(vk(KeyCode::KEY_ESC), crate::hotkey::VK_ESCAPE);
        assert_eq!(vk(KeyCode::KEY_F13), crate::hotkey::VK_F13);
        assert!(vk(KeyCode::KEY_A) > 0xFFFF);
    }

    #[test]
    fn ascii_types_on_the_us_layout() {
        assert_eq!(keys_for('a'), Some(vec![KeyCode::KEY_A]));
        assert_eq!(keys_for('A'), Some(vec![KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_A]));
        assert_eq!(keys_for('('), Some(vec![KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_9]));
        assert_eq!(keys_for('7'), Some(vec![KeyCode::KEY_7]));
        assert_eq!(keys_for('é'), None);
    }
}
