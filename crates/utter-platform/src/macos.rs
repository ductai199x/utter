//! macOS: the platform layer through the system's own frameworks (docs/plan-cross-platform.md,
//! phase 2). Compile-checked in CI; not yet run on a Mac.
//!
//! - Keys: one listen-only event tap (CoreGraphics) for every subscriber, the hotkey and the
//!   paste-last shortcuts, as linux_input does with evdev. It takes the Input Monitoring
//!   permission.
//! - Paste: the general pasteboard (AppKit), then Cmd+V posted as CoreGraphics key events, which
//!   takes the Accessibility permission.
//! - Focus: the frontmost application (NSWorkspace), its pid as the window.
//! - Caret: the focused element's selected range and the text before it (Accessibility).
//! - GPU: the system's default Metal device.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;

use crate::hotkey::KeyEvent;

// CoreFoundation, CoreGraphics and Accessibility: C APIs, declared here.
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFRunLoopRef = *mut c_void;
type CGEventRef = *mut c_void;
type AXUIElementRef = *const c_void;

type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[repr(C)]
#[derive(Clone, Copy)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: *mut c_void, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventGetTimestamp(event: CGEventRef) -> u64;
    fn CGEventCreateKeyboardEvent(source: CFTypeRef, keycode: u16, down: bool) -> CGEventRef;
    fn CGEventKeyboardSetUnicodeString(event: CGEventRef, length: usize, string: *const u16);
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
}

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut AnyObject;
}

// CoreGraphics constants (CGEventTypes.h).
const SESSION_TAP: u32 = 1;
const HID_TAP: u32 = 0;
const HEAD_INSERT: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const FIELD_AUTOREPEAT: u32 = 8;
const FIELD_KEYCODE: u32 = 9;
const FIELD_USER_DATA: u32 = 42;
const FLAG_SHIFT: u64 = 0x2_0000;
const FLAG_CONTROL: u64 = 0x4_0000;
const FLAG_OPTION: u64 = 0x8_0000;
const FLAG_COMMAND: u64 = 0x10_0000;
const FLAG_FN: u64 = 0x80_0000;
/// Marks the key events Utter posts, so the tap knows them for its own.
const OURS: i64 = 0x5554_5452; // "UTTR"
const AX_RANGE: u32 = 4; // kAXValueCFRangeType

/// A Mac key code as the Windows virtual-key code the rest of Utter speaks (hotkey.rs); keys
/// without one are 0x10000 + the code.
pub fn vk(code: u16) -> u32 {
    match code {
        0x3E => 0xA3, // Right Control
        0x3B => 0xA2, // Left Control
        0x3C => 0xA1, // Right Shift
        0x38 => 0xA0, // Left Shift
        0x3D => 0xA5, // Right Option
        0x3A => 0xA4, // Left Option
        0x37 => 0x5B, // Left Command
        0x36 => 0x5C, // Right Command
        0x35 => 0x1B, // Escape
        0x69 => 0x7C, // F13
        0x6B => 0x7D,
        0x71 => 0x7E,
        0x6A => 0x7F,
        0x40 => 0x80,
        0x4F => 0x81,
        0x50 => 0x82,
        0x5A => 0x83, // F20
        0x09 => 0x56, // V
        0x08 => 0x43, // C
        0x06 => 0x5A, // Z
        0x07 => 0x58, // X
        other => 0x1_0000 + u32::from(other),
    }
}

/// A modifier key's own bit in the event flags (IOLLEvent.h's device masks): whether it is down
/// after a flags-changed event. None for other keys.
fn device_mask(code: u16) -> Option<u64> {
    Some(match code {
        0x3B => 0x0001,
        0x38 => 0x0002,
        0x3C => 0x0004,
        0x37 => 0x0008,
        0x36 => 0x0010,
        0x3A => 0x0020,
        0x3D => 0x0040,
        0x3E => 0x2000,
        0x3F => FLAG_FN,
        _ => return None,
    })
}

/// Modifiers held now, as bits (linux_input's): Control, Shift, Option, Command.
static MODIFIERS: AtomicU8 = AtomicU8::new(0);
const CTRL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;
const META: u8 = 8;

fn modifier_bits(flags: u64) -> u8 {
    let mut bits = 0;
    for (flag, bit) in [(FLAG_CONTROL, CTRL), (FLAG_SHIFT, SHIFT), (FLAG_OPTION, ALT), (FLAG_COMMAND, META)] {
        if flags & flag != 0 {
            bits |= bit;
        }
    }
    bits
}

pub fn modifiers_down() -> bool {
    MODIFIERS.load(Ordering::Relaxed) != 0
}

/// A key event with what the shortcuts need besides.
#[derive(Clone, Copy)]
struct Key {
    event: KeyEvent,
    code: u16,
    repeat: bool,
    modifiers: u8,
}

static SUBSCRIBERS: Mutex<Vec<Sender<Key>>> = Mutex::new(Vec::new());
static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

extern "C" fn on_event(_proxy: *mut c_void, kind: u32, event: CGEventRef, _user: *mut c_void) -> CGEventRef {
    if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER_INPUT {
        // The system turns a slow tap off; this one only listens, so turn it back on.
        let tap = TAP.load(Ordering::Relaxed);
        if !tap.is_null() {
            // SAFETY: the tap this process made, alive for the process.
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }
    // SAFETY: reading fields of the event the system passed in.
    let (code, flags, repeat, user, time) = unsafe {
        (
            CGEventGetIntegerValueField(event, FIELD_KEYCODE) as u16,
            CGEventGetFlags(event),
            CGEventGetIntegerValueField(event, FIELD_AUTOREPEAT) != 0,
            CGEventGetIntegerValueField(event, FIELD_USER_DATA),
            CGEventGetTimestamp(event),
        )
    };
    let modifiers = modifier_bits(flags);
    MODIFIERS.store(modifiers, Ordering::Relaxed);
    let down = match kind {
        KEY_DOWN => true,
        KEY_UP => false,
        FLAGS_CHANGED => match device_mask(code) {
            Some(mask) => flags & mask != 0,
            None => return event,
        },
        _ => return event,
    };
    let key = Key {
        event: KeyEvent { vk: vk(code), down, time_ms: (time / 1_000_000) as u32, injected: user == OURS },
        code,
        repeat,
        modifiers,
    };
    SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner()).retain(|tx| tx.send(key).is_ok());
    event
}

/// Starts the event tap on its own thread (once): Err when macOS refuses it, which means the
/// Input Monitoring permission is missing.
fn start_tap() -> Result<(), String> {
    static STARTED: Mutex<bool> = Mutex::new(false);
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if *started {
        return Ok(());
    }
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("utter-keys-tap".into())
        .spawn(move || {
            let mask = (1u64 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED);
            // SAFETY: CoreGraphics and CoreFoundation calls with valid arguments; the tap and
            // its source live as long as the process, on this thread's run loop.
            unsafe {
                let tap = CGEventTapCreate(SESSION_TAP, HEAD_INSERT, LISTEN_ONLY, mask, on_event, std::ptr::null_mut());
                if tap.is_null() {
                    let _ = ready_tx.send(false);
                    return;
                }
                TAP.store(tap, Ordering::Relaxed);
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);
                let _ = ready_tx.send(true);
                CFRunLoopRun();
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(true) => {
            *started = true;
            Ok(())
        }
        _ => Err("macOS did not let Utter read the keyboard (Input Monitoring, in Privacy & Security)".into()),
    }
}

fn subscribe() -> Result<Receiver<Key>, String> {
    start_tap()?;
    let (tx, rx) = mpsc::channel();
    SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
    Ok(rx)
}

/// Forwards `rx` to `f` on a thread until the returned stop runs.
fn forward(
    rx: Receiver<Key>,
    name: &str,
    mut f: impl FnMut(Key) -> bool + Send + 'static,
) -> Result<Box<dyn FnMut() + Send>, String> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(key) => {
                        if !f(key) {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Box::new(move || stop.store(true, Ordering::Relaxed)))
}

/// The hotkey's key events (hotkey.rs).
pub fn start_hook(tx: Sender<KeyEvent>) -> Result<Box<dyn FnMut() + Send>, String> {
    forward(subscribe()?, "utter-keys-mac", move |k| tx.send(k.event).is_ok())
}

/// Ctrl+Cmd+V and Ctrl+Cmd+C, pressed with exactly those modifiers (shortcut.rs).
pub fn register_shortcuts(
    mut on: Box<dyn FnMut(crate::shortcut::Shortcut) + Send>,
) -> Result<Box<dyn FnMut() + Send>, String> {
    use crate::shortcut::Shortcut;
    forward(subscribe()?, "utter-shortcuts", move |k| {
        if k.event.down && !k.repeat && !k.event.injected && k.modifiers == CTRL | META {
            match k.code {
                0x09 => on(Shortcut::PasteLast),
                0x08 => on(Shortcut::CopyLast),
                _ => {}
            }
        }
        true
    })
}

/// Whether Utter has what it needs for keys: Input Monitoring (to read them) and Accessibility
/// (to post the paste keys and read the caret).
pub fn input_access() -> Result<(), String> {
    // SAFETY: plain queries.
    let (listen, trusted) = unsafe { (CGPreflightListenEventAccess(), AXIsProcessTrusted()) };
    match (listen, trusted) {
        (true, true) => Ok(()),
        (false, true) => Err("Input Monitoring is off for Utter".into()),
        (true, false) => Err("Accessibility is off for Utter".into()),
        (false, false) => Err("Input Monitoring and Accessibility are off for Utter".into()),
    }
}

/// Whether Utter's Accessibility switch is on, as of now (input_access's view of reading keys
/// is the process's, fixed until it restarts).
pub fn accessibility_on() -> bool {
    // SAFETY: a plain query.
    unsafe { AXIsProcessTrusted() }
}

/// Asks macOS for what input_access found missing, Accessibility first: its prompt adds Utter to
/// the list in Privacy & Security, and once it is on, reading keys is allowed too (macOS 26.7:
/// Input Monitoring then needs nothing more). Asking for both at once added Utter to neither
/// list. Opens the pane to turn on; Ok means it is open, not that the switch is on.
pub fn allow_input() -> Result<(), String> {
    // SAFETY: CoreFoundation calls with valid arguments; the dictionary is released.
    let (listen, trusted) = unsafe {
        let trusted_now = AXIsProcessTrusted();
        let listen = CGPreflightListenEventAccess() || (trusted_now && CGRequestListenEventAccess());
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        let trusted = AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
        (listen, trusted)
    };
    let pane = match (listen, trusted) {
        (true, true) => return Ok(()),
        (_, false) => "Privacy_Accessibility",
        (false, true) => "Privacy_ListenEvent",
    };
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
    std::process::Command::new("open").arg(url).spawn().map_err(|e| format!("could not open System Settings: {e}"))?;
    Ok(())
}

/// Posts key `code` down and up with `flags`, marked as Utter's.
fn post_key(code: u16, flags: u64) -> Result<(), String> {
    for down in [true, false] {
        // SAFETY: an event made, posted and released here.
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), code, down);
            if event.is_null() {
                return Err("CoreGraphics made no key event".into());
            }
            CGEventSetFlags(event, flags);
            CGEventSetIntegerValueField(event, FIELD_USER_DATA, OURS);
            CGEventPost(HID_TAP, event);
            CFRelease(event);
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}

/// Types `text` as Unicode key events: exact for any character, whatever the layout.
fn type_unicode(text: &str) -> Result<(), String> {
    for ch in text.chars() {
        if ch == '\n' {
            post_key(0x24, 0)?; // Return
            continue;
        }
        let mut units = [0u16; 2];
        let units = ch.encode_utf16(&mut units);
        for down in [true, false] {
            // SAFETY: an event made, posted and released here; `units` outlives the call.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), 0, down);
                if event.is_null() {
                    return Err("CoreGraphics made no key event".into());
                }
                CGEventKeyboardSetUnicodeString(event, units.len(), units.as_ptr());
                CGEventSetIntegerValueField(event, FIELD_USER_DATA, OURS);
                CGEventPost(HID_TAP, event);
                CFRelease(event);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn ns_string(s: &NSString) -> String {
    s.to_string()
}

/// The text before the caret in the focused element, up to `chars` characters.
pub fn before_caret(chars: i32) -> Option<String> {
    // SAFETY: Accessibility and CoreFoundation calls; every copied value is released, and each
    // is checked for its type before use.
    unsafe {
        let system = AXUIElementCreateSystemWide();
        let focused_attr = NSString::from_str("AXFocusedUIElement");
        let mut focused: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(system, Retained::as_ptr(&focused_attr).cast(), &mut focused);
        CFRelease(system);
        if err != 0 || focused.is_null() {
            return None;
        }
        let range_attr = NSString::from_str("AXSelectedTextRange");
        let mut range_value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(focused, Retained::as_ptr(&range_attr).cast(), &mut range_value);
        let mut range = CFRange { location: 0, length: 0 };
        let ok = err == 0
            && !range_value.is_null()
            && AXValueGetValue(range_value, AX_RANGE, (&mut range as *mut CFRange).cast());
        if !range_value.is_null() {
            CFRelease(range_value);
        }
        if !ok {
            CFRelease(focused);
            return None;
        }
        let start = (range.location - chars as isize).max(0);
        let wanted = CFRange { location: start, length: range.location - start };
        let param = AXValueCreate(AX_RANGE, (&wanted as *const CFRange).cast());
        let for_range = NSString::from_str("AXStringForRange");
        let mut text: CFTypeRef = std::ptr::null();
        let err =
            AXUIElementCopyParameterizedAttributeValue(focused, Retained::as_ptr(&for_range).cast(), param, &mut text);
        CFRelease(param);
        CFRelease(focused);
        if err != 0 || text.is_null() {
            return None;
        }
        if CFGetTypeID(text) != CFStringGetTypeID() {
            CFRelease(text);
            return None;
        }
        // A CFString is an NSString (toll-free bridged); this takes over the copy's reference.
        let s: Retained<NSString> = Retained::from_raw(text as *mut NSString)?;
        Some(ns_string(&s))
    }
}

/// The system's default Metal device: Apple Silicon's GPU, which shares the system's memory.
/// Its memory is the working set Metal recommends for one process.
pub fn gpu_adapters() -> Vec<crate::gpu::Adapter> {
    // SAFETY: Metal's documented calls on the device it returns (+1, released by Retained).
    unsafe {
        let Some(device) = Retained::from_raw(MTLCreateSystemDefaultDevice()) else { return Vec::new() };
        let name: Option<Retained<NSString>> = msg_send![&*device, name];
        let working_set: u64 = msg_send![&*device, recommendedMaxWorkingSetSize];
        let allocated: usize = msg_send![&*device, currentAllocatedSize];
        let registry: u64 = msg_send![&*device, registryID];
        vec![crate::gpu::Adapter {
            name: name.map(|n| ns_string(&n)).unwrap_or_else(|| "Apple GPU".into()),
            vendor_id: 0x106B,
            device_id: 0,
            luid: (registry as u32, (registry >> 32) as i32),
            memory_bytes: working_set,
            free_bytes: Some(working_set.saturating_sub(allocated as u64)),
        }]
    }
}

pub mod focus {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};

    use crate::focus::{App, Window};

    fn app_of(app: &NSRunningApplication) -> App {
        let exe = app.executableURL().and_then(|u| u.lastPathComponent()).map(|s| s.to_string()).unwrap_or_default();
        let name = app.localizedName().map(|s| s.to_string()).unwrap_or_else(|| exe.clone());
        App { exe, name }
    }

    pub fn foreground_handle() -> Option<isize> {
        NSWorkspace::sharedWorkspace().frontmostApplication().map(|a| a.processIdentifier() as isize)
    }

    pub fn foreground() -> Option<Window> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        let app = app_of(&app);
        Some(Window { handle: pid as isize, process_id: pid as u32, title: app.name.clone(), app })
    }

    pub fn refuses_input(_pid: u32) -> bool {
        false
    }

    /// The apps in the Dock (regular activation policy): the ones with windows people use.
    pub fn open_apps() -> Vec<App> {
        NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
            .map(|a| app_of(&a))
            .collect()
    }
}

pub mod insert {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    use crate::insert::{DefaultRule, InsertError, Keep, PasteKeys, PasteRule};

    const WAIT_3S: PasteRule = PasteRule { keys: PasteKeys::Primary, keep: Keep::Seconds3 };
    const WAIT_5S: PasteRule = PasteRule { keys: PasteKeys::Primary, keep: Keep::Seconds5 };

    /// Apps that read the pasteboard late (remote desktops send it over the network first).
    /// Terminals paste with Cmd+V on macOS, like every other app.
    pub const DEFAULT_RULES: &[DefaultRule] = &[
        ("Microsoft Remote Desktop", "Microsoft Remote Desktop", WAIT_5S),
        ("Windows App", "Windows App", WAIT_5S),
        ("Citrix Viewer", "Citrix Workspace", WAIT_5S),
        ("TeamViewer", "TeamViewer", WAIT_5S),
        ("VNC Viewer", "VNC Viewer", WAIT_5S),
        ("Microsoft Word", "Microsoft Word", WAIT_3S),
        ("Microsoft Outlook", "Microsoft Outlook", WAIT_3S),
    ];

    /// The pasteboard's items, each as its types and their data: all of it, as Windows saves
    /// every clipboard format. Plain bytes, so the restore thread can hold it.
    pub type Snapshot = Vec<Vec<(String, Vec<u8>)>>;

    /// Clipboard managers leave items with these types out of their history (nspasteboard.org).
    const TRANSIENT: &str = "org.nspasteboard.TransientType";
    const AUTO_GENERATED: &str = "org.nspasteboard.AutoGeneratedType";

    fn pasteboard() -> Retained<NSPasteboard> {
        NSPasteboard::generalPasteboard()
    }

    pub fn save_all() -> Result<Snapshot, InsertError> {
        let pb = pasteboard();
        let Some(items) = pb.pasteboardItems() else { return Ok(Vec::new()) };
        Ok(items
            .iter()
            .map(|item| {
                item.types().iter().filter_map(|t| item.dataForType(&t).map(|d| (t.to_string(), d.to_vec()))).collect()
            })
            .collect())
    }

    fn write(pb: &NSPasteboard, items: Vec<Retained<NSPasteboardItem>>) -> Result<(), InsertError> {
        pb.clearContents();
        if items.is_empty() {
            return Ok(());
        }
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            items.into_iter().map(ProtocolObject::from_retained).collect();
        if pb.writeObjects(&NSArray::from_retained_slice(&objects)) {
            Ok(())
        } else {
            Err(InsertError::Clipboard("the pasteboard refused the text".into()))
        }
    }

    pub fn put_text(text: &str, transient: bool) -> Result<u32, InsertError> {
        let pb = pasteboard();
        let item = NSPasteboardItem::new();
        // SAFETY: the extern static AppKit defines.
        let string_type = unsafe { NSPasteboardTypeString };
        item.setString_forType(&NSString::from_str(text), string_type);
        if transient {
            let empty = NSData::new();
            item.setData_forType(&empty, &NSString::from_str(TRANSIENT));
            item.setData_forType(&empty, &NSString::from_str(AUTO_GENERATED));
        }
        write(&pb, vec![item])?;
        Ok(pb.changeCount() as u32)
    }

    pub fn sequence() -> u32 {
        pasteboard().changeCount() as u32
    }

    pub fn is_free() -> bool {
        true
    }

    pub fn press(keys: PasteKeys) -> Result<(), InsertError> {
        use super::{FLAG_COMMAND, FLAG_SHIFT, post_key};
        let (code, flags) = match keys {
            PasteKeys::Primary => (0x09, FLAG_COMMAND),
            PasteKeys::CtrlShiftV => (0x09, FLAG_COMMAND | FLAG_SHIFT),
            PasteKeys::ShiftInsert => (0x72, FLAG_SHIFT), // Help/Insert
            PasteKeys::Type => return Err(InsertError::Input("typing goes through type_text".into())),
        };
        post_key(code, flags).map_err(InsertError::Input)
    }

    pub fn restore(saved: Snapshot, ours: u32) {
        let pb = pasteboard();
        if pb.changeCount() as u32 != ours {
            tracing::debug!("the pasteboard changed since the paste; not restoring");
            return;
        }
        let items = saved
            .into_iter()
            .map(|types| {
                let item = NSPasteboardItem::new();
                for (t, d) in types {
                    item.setData_forType(&NSData::with_bytes(&d), &NSString::from_str(&t));
                }
                item
            })
            .collect();
        if let Err(e) = write(&pb, items) {
            tracing::warn!("restoring the pasteboard: {e}");
        }
    }

    pub fn clipboard_text() -> Option<String> {
        // SAFETY: the extern static AppKit defines.
        pasteboard().stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string())
    }

    pub fn type_text(text: &str) -> Result<(), InsertError> {
        super::type_unicode(text).map_err(InsertError::Input)
    }

    pub fn warm() {}
}
