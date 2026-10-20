# The Bar

The small floating pill that shows Utter is listening. It is the only part of Utter most
people see in a day, so every state, motion and pixel here is deliberate. Status for 0.1.11:
**built** (`app/src/bar`, `app/src-tauri/src/main.rs`, `crates/utter-platform/src/overlay.rs`).
The checks at the end say what is shown by code, tests or measurements, and what is still
unverified. Sound cues were considered and dropped (2026-10-20): the Bar has no sounds.

## What it is for

1. Confirm, instantly, that Utter heard the key and the microphone is live.
2. Show that the voice is being picked up (the waveform) and what is being understood (live
   words).
3. Get out of the way: it never takes the focus, never blocks a click, and is gone when not
   needed.

## Where

- Bottom center of the monitor that holds the focused window, 48 px (logical) above the work
  area's bottom edge (above the taskbar). The window is placed once, when the Bar shows; the
  monitor does not change during a dictation.
- Moving and docking it is a later feature.

## States

The page (`Bar.svelte`) draws the state the core sends (`bar:state`) and the words it sends
(`bar:words`); Rust owns every timer.

| State | Shown when | Looks like | Leaves to |
|---|---|---|---|
| Hidden | idle | nothing | Listening (key down) |
| Listening | key held, or hands-free | pill with live waveform; live words to its right once there are any | Finishing (key up), Cancelled |
| Hands-free | after a double tap | as Listening, plus a 5 px red dot at the left edge | Finishing (tap), Cancelled |
| Finishing | key up until the text is pasted | waveform bars fall to a calm baseline; a soft shimmer sweeps the pill after 150 ms (only if it is still finishing then) | Done, Error |
| Done | text pasted | a check mark draws in (220 ms) and stays 400 ms, with the pasted words beside it; then the pill fades out | Hidden |
| Cancelled | chord, short tap or Esc | no check; fades out at once (160 ms) | Hidden |
| Error | nothing heard, text could not be pasted, dictation stopped | one line of amber text; 2.5 s | Hidden |
| Notice | an event outside a dictation (see below) | a check (or an amber dot) and one line | Hidden after 1.2 s (2.5 s for an amber one) |

Error lines (`NotInserted::message` in `crates/utter-pipeline/src/lib.rs`, `main.rs`):

| Cause | Line |
|---|---|
| Nothing recognized | "Didn't catch that" |
| The microphone gave no sound | "No sound from the microphone: is it connected and unmuted?" |
| Focus moved, no window, paste failed (text copied) | "Copied: press Ctrl+V to paste" (the paste keys of the OS) |
| App runs as administrator | "Copied: this app runs as administrator. Press Ctrl+V" |
| App ignored the paste | "It didn't paste here. Copied: press Ctrl+V" |
| Neither paste nor copy worked | "Couldn't paste or copy it: it is in History" |
| The pipeline stopped | "Dictation stopped" |

A Notice is a short message when no dictation is running: "Pasted", "Copied" and "Nothing to
paste yet" from the tray or the shortcuts (docs/ux/insertion.md), "Microphone connected" and
"Microphone disconnected", "No microphone: connect one, or choose one in Settings". A check
means it did what was asked; an amber dot and amber text mean it did not.

A tap that is cancelled quickly (a chord such as Right Ctrl + C, or a stray tap) must not flash a Bar. The window is shown 120 ms after key down (`BAR_GRACE` in `app/src-tauri/src/main.rs`); a cancel before that means it never appears. Recording starts at key down regardless, and a double-tap's hands-free Bar shows at once.

## Geometry (logical pixels; scaled by the monitor's DPI)

- Pill height 40 for one line; corner radius 20 (a full pill when one line high).
- Width 124 with waveform only (16 bars plus 14 px padding each side). With words it grows to
  fit them on one line, up to 520. Past 520 the words wrap, as text does: the pill grows
  upward one line (20 px) at a time, its bottom edge fixed, up to 5 lines (120 px). Past 5
  lines the oldest scroll off the top: nothing shows in the top padding, and the oldest full
  line fades in from 35%. The newest line is always the bottom one.
- Waveform (and the Done check, and the notice dot): in a 38 px box at the bottom left, so it
  sits beside the newest line. 16 bars, 3 wide, 3 apart (93 px), heights 3 to 22. A notice's
  slot is 18 px wide. An error line has no slot and the pill is 520 wide.
- Words: 14 px on a 20 px line, 12 px to the right of the waveform, 16 px padding at the end,
  9 px above and below.
- The window is 576 x 168: the tallest pill, its shadow, and the 12 px below it.

## Motion

All durations short; nothing ever blocks input. One set of easing tokens
(`app/src/lib/tokens.css`):

| Token | Value | Use |
|---|---|---|
| `--ease-out` | `cubic-bezier(0.2, 0, 0, 1)` | entrances, growth |
| `--ease-in` | `cubic-bezier(0.4, 0, 1, 1)` | exits |
| `--dur-enter` | 120 ms | appear: opacity 0 -> 1, scale 0.97 -> 1, from the bottom center |
| `--dur-exit` | 160 ms | disappear: opacity 1 -> 0, scale 1 -> 0.97 |
| `--dur-resize` | 180 ms | width and height changes as words arrive |
| `--dur-check` | 400 ms | how long the Done check shows (Rust `DONE_HOLD`) |
| `--dur-word` | 120 ms | provisional words fade in; word color changes |

Waveform (`app/src/lib/waveform.ts`): driven by the microphone level, sent every 16 ms, and
drawn at the display's refresh rate. Each bar follows the peak level of its own slice of the
last 600 ms (newest on the right). Attack 30 ms, release 140 ms (rises fast, falls gently), so
speech looks lively and silence looks still, not dead: idle bars rest at 3 px. The loop stops
while the Bar is hidden.

Live words arrive in pieces (each segment, and the preview of the segment being spoken). The
page lays the text out at the pill's target width, so as the pill grows it reveals the text
instead of reflowing it. The provisional words update in place, so the fade does not replay on
every recognizer pass.

What you see is what is pasted. At every pause the dictation so far is cleaned (punctuation,
symbols, numbers, corrections applied) and shown at full weight; the words heard after it are
provisional (dimmed, `--bar-words-provisional`) until the next cleanup replaces them. A later
correction can still change cleaned text ("five, no wait, seven"). At key up the cleaned whole
is the text pasted (`Event::Final` carries the same `text` that is inserted), and it stays
under the Done check.

## Color

The Bar is always dark, whatever the theme: it floats over arbitrary content and must read on
white and black alike.

| Token | Value |
|---|---|
| background | `rgba(18, 18, 20, 0.92)` |
| border | 1 px `rgba(255, 255, 255, 0.08)` |
| shadow | `0 8px 24px rgba(0, 0, 0, 0.28)` |
| waveform | `rgba(255, 255, 255, 0.92)` |
| words being pasted | `rgba(255, 255, 255, 0.86)` |
| provisional words | `rgba(255, 255, 255, 0.5)` |
| hands-free dot | `#ff453a` |
| error text | `#ffb340` |

## Sound

Utter makes no sound: no cue at start, stop, cancel or error, and no setting for one. The error text says enough, and the Bar already shows every state. (Dropped 2026-10-20.)

## Focus and input rules (non-negotiable)

- The Bar window never becomes the foreground window and never takes keyboard focus.
  Windows: `WS_EX_NOACTIVATE` and `SW_SHOWNOACTIVATE`-style placement (`SetWindowPos` with
  `SWP_NOACTIVATE`, `crates/utter-platform/src/overlay.rs`), and `"focusable": false` in
  `tauri.conf.json`. Linux: a notification-type window hint, so the window manager never
  activates it.
- It ignores the mouse entirely: clicks pass through to whatever is under it (Tauri's
  `set_ignore_cursor_events(true)`, which manages the layered-window style).
- No taskbar button, no Alt+Tab entry (`WS_EX_TOOLWINDOW`, `skipTaskbar`).
- It is created once at startup, hidden, with its page loaded: showing it is a native call
  plus a state change, never a page load. The core sends the state first, then shows the
  window, so the page has reset before it becomes visible. The core never waits for the UI
  thread to show it.

## Timing budgets

| From | To | Budget | Measured |
|---|---|---|---|
| key down | Bar's first visible frame | 120 ms grace (`BAR_GRACE`) plus under 16 ms, so a chord never shows it | not measured |
| a level change | the waveform drawing it | one frame | by design (level every 16 ms, drawn each frame); not measured |
| a segment transcribed | its words visible | one frame | not measured |
| paste sent | Done check visible | one frame | not measured |

The measured pipeline times for a dictation (key up to text in the app, 231-260 ms with the
CPU-thread setting in use; 2026-10-12, Quality, RTX 3090 Ti) are in docs/evals.md. They do not
include the Bar.

## Acceptance checks

Each is to be checked on the real machine, at 100% and 150% display scaling, light and dark
Windows themes, over a white page and a black terminal. "Code" means the behavior is in the
source; "unverified" means no test or measurement in the repo shows it.

| # | Check | Status |
|---|---|---|
| 1 | Pressing Right Ctrl shows the Bar 120 ms later (one frame after the grace); it never steals the focus (the caret in the target app keeps blinking; typing after the dictation goes to the target) | Focus: code (no-activate styles, `focusable: false`); the timing: unverified |
| 2 | Clicking where the Bar is clicks the app under it | Code (`set_ignore_cursor_events`); unverified on screen |
| 3 | The waveform moves with the voice and rests still at 3 px in silence; no stutter during transcription work | Code; the CPU-thread work in docs/evals.md (2026-10-12) removed microphone gaps; the waveform itself unverified |
| 4 | Live words appear while speaking; words already shown never jump | Code (committed and preview text, laid out at the target width); unverified on screen |
| 5 | Right Ctrl + C copies and the Bar never appears | Copy: met (the chord cancels, test in `hotkey.rs`). "Never appears": code (`BAR_GRACE` delays the show past a quick cancel); unverified on screen |
| 6 | Esc cancels with no check, no sound | Met in code and in `hotkey.rs` tests (Esc is a cancel; no sound exists) |
| 7 | A long dictation wraps into up to 5 lines, the newest at the bottom; older lines scroll off the top under a fade | Code (`MAX_LINES`, mask); unverified on screen |
| 8 | Done shows the check briefly and hides; nothing lingers | Code (400 ms hold, 160 ms exit, then the window hides); unverified on screen |
| 9 | An error line shows when the text could not be pasted (target closed or changed) | Code (lines in the table above); the original wording "Focus moved: text kept in History" is replaced by "Copied: press Ctrl+V to paste" |
| 10 | Text is crisp at every scale; the pill's edge is anti-aliased, no seams or halos on white | Unverified |
| 11 | The text under the Done check is exactly the text that was pasted | Met in code (`Event::Final` and the insert use the same `text`) |
