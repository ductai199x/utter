//! The text just before the cursor in the focused app, so a dictation that continues text can
//! be joined to it with a space (docs/ux/insertion.md, "Spacing"), and a paste can be seen to
//! have landed ("Confirming the paste"). Windows reads it through UI
//! Automation's text pattern, which Notepad, Word, browsers' text fields and most editors
//! expose; an app that does not (a terminal) gives nothing, and Utter adds nothing.
//!
//! Asked at key down and answered by a thread of its own while the person speaks, so it is
//! never on the way from key up to the paste: a slow app only means no answer in time.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// How many characters before the cursor spacing needs.
pub const BEFORE_CHARS: i32 = 2;

/// An answer on its way ([`before_cursor`]).
pub struct Pending(Receiver<Option<String>>);

impl Pending {
    /// The text before the cursor (`Some("")` at the start of the field), if it came within
    /// `wait`; `None` if the app does not say or did not say in time.
    pub fn get(&self, wait: Duration) -> Option<String> {
        self.0.recv_timeout(wait).ok().flatten()
    }
}

/// Starts reading up to `chars` characters before the cursor of the focused control.
pub fn before_cursor(chars: i32) -> Pending {
    let (tx, rx) = mpsc::channel();
    match worker() {
        Some(w) => {
            let _ = w.lock().expect("caret worker lock").send((chars, tx));
        }
        None => {
            let _ = tx.send(None);
        }
    }
    Pending(rx)
}

/// Where queries go: each carries how many characters to read and where its answer goes.
type Requests = Sender<(i32, Sender<Option<String>>)>;

/// One thread for every query: COM set up once, the automation object made once (tens of
/// milliseconds), and never on a thread that owns windows or hooks.
fn worker() -> Option<&'static Mutex<Requests>> {
    static WORKER: OnceLock<Option<Mutex<Requests>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<(i32, Sender<Option<String>>)>();
            std::thread::Builder::new()
                .name("utter-caret".into())
                .spawn(move || {
                    let reader = imp::Reader::new();
                    for (chars, reply) in rx {
                        let _ = reply.send(reader.as_ref().and_then(|r| r.before_cursor(chars)));
                    }
                })
                .ok()
                .map(|_| Mutex::new(tx))
        })
        .as_ref()
}

/// Starts the worker now, so the first dictation does not pay for setting it up.
pub fn warm() {
    let _ = worker();
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, TextPatternRangeEndpoint_End,
        TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_TextPatternId,
    };

    pub struct Reader(IUIAutomation);

    impl Reader {
        pub fn new() -> Option<Self> {
            // SAFETY: COM for this thread (it lives as long as the thread), then one automation
            // object, used only on this thread.
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
                match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(a) => Some(Self(a)),
                    Err(e) => {
                        tracing::warn!("UI Automation unavailable: {e}");
                        None
                    }
                }
            }
        }

        /// Up to `chars` characters before the start of the focused control's selection (the
        /// cursor, when nothing is selected).
        pub fn before_cursor(&self, chars: i32) -> Option<String> {
            // SAFETY: plain UI Automation calls on objects this thread created or received.
            unsafe {
                let element = self.0.GetFocusedElement().ok()?;
                let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
                let selection = pattern.GetSelection().ok()?;
                if selection.Length().ok()? < 1 {
                    return None;
                }
                let at = selection.GetElement(0).ok()?;
                let before = at.Clone().ok()?;
                // Collapse to the cursor, then reach back.
                before.MoveEndpointByRange(TextPatternRangeEndpoint_End, &at, TextPatternRangeEndpoint_Start).ok()?;
                before.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -chars).ok()?;
                Some(before.GetText(chars).ok()?.to_string())
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// macOS: the focused element's AXSelectedTextRange and AXStringForRange; Linux: AT-SPI's
    /// Text interface (caret offset, get_text). Built with those platforms.
    pub struct Reader;

    impl Reader {
        pub fn new() -> Option<Self> {
            None
        }

        pub fn before_cursor(&self, _chars: i32) -> Option<String> {
            None
        }
    }
}
