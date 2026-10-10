# The Bar

The small floating pill that shows Utter is listening. It is the only part of Utter most
people see in a day, so every state, motion and pixel here is deliberate. Status: **spec, not
built**. The implementation is reviewed against the checks at the end.

## What it is for

1. Confirm, instantly, that Utter heard the key and the microphone is live.
2. Show that the voice is being picked up (the waveform) and what is being understood (live
   words).
3. Get out of the way: it never takes the focus, never blocks a click, and is gone when not
   needed.

## Where

- Bottom center of the monitor that holds the focused window, 48 px (logical) above the work
  area's bottom edge (above the taskbar).
- On the same monitor for the whole dictation, even if the focus changes.
- Moving and docking it is a later feature.

## States

| State | Shown when | Looks like | Leaves to |
|---|---|---|---|
| Hidden | idle | nothing | Listening (key down) |
| Listening | key held, or hands-free | pill with live waveform; live words to its right once there are any | Finishing (key up), Cancelled |
| Hands-free | after a double tap | as Listening, plus a small red dot at the left edge | Finishing (tap), Cancelled |
| Finishing | key up until the text is pasted | waveform bars fall to a calm baseline; a soft shimmer sweeps the pill after 150 ms (only if it is still finishing then) | Done, Error |
| Done | text pasted | a check mark replaces the waveform for 400 ms | Hidden |
| Cancelled | chord, short tap or Esc | no check; fades out at once | Hidden |
| Error | nothing heard, focus moved, paste blocked | one line of amber text, e.g. "Focus moved: text kept in History" | Hidden after 2.5 s |

Nothing appears for a tap that is cancelled within 150 ms (a chord such as Right Ctrl + C, or a
stray tap): the Bar starts its entrance at once but the cancel arrives before it is visible,
and it simply never shows. Showing then hiding a flash would be worse.

## Geometry (logical pixels; scaled by the monitor's DPI)

- Pill height 40; corner radius 20 (a full pill).
- Width 124 with waveform only (16 bars plus 14 px padding each side). With live words it grows to fit them, up to 520; longer text
  keeps its newest words visible and fades the oldest out at the left edge (a 32 px mask
  gradient).
- Waveform: 16 bars, 3 wide, 3 apart (93 px), centered vertically, heights 3 to 22.
- Live words: 14 px, one line, 12 px to the right of the waveform, 16 px padding at the end.

## Motion

All durations short; nothing ever blocks input. One set of easing tokens:

| Token | Value | Use |
|---|---|---|
| `--ease-out` | `cubic-bezier(0.2, 0, 0, 1)` | entrances, growth |
| `--ease-in` | `cubic-bezier(0.4, 0, 1, 1)` | exits |
| `--dur-enter` | 120 ms | appear: opacity 0 -> 1, scale 0.94 -> 1, from the bottom center |
| `--dur-exit` | 160 ms | disappear: opacity 1 -> 0, scale 1 -> 0.97 |
| `--dur-resize` | 180 ms | width changes as words arrive |
| `--dur-check` | 400 ms | the Done check |

Waveform: driven by the microphone level at the display's refresh rate. Each bar follows the
level of its own slice of the last 600 ms (newest on the right). Attack 30 ms, release 140 ms
(rises fast, falls gently), so speech looks lively and silence looks still, not dead: idle bars
rest at 3 px.

Live words arrive in pieces (each segment, and the preview of the segment being spoken). New
words fade in over 120 ms. Words already shown never jump: a preview may only extend or
correct its own tail, and a correction cross-fades the changed words in place.

## Color

The Bar is always dark, whatever the theme: it floats over arbitrary content and must read on
white and black alike.

| Token | Value |
|---|---|
| background | `rgba(18, 18, 20, 0.92)` |
| border | 1 px `rgba(255, 255, 255, 0.08)` |
| shadow | `0 8px 24px rgba(0, 0, 0, 0.28)` |
| waveform | `rgba(255, 255, 255, 0.92)` |
| live words | `rgba(255, 255, 255, 0.86)`; the newest word `0.98` |
| hands-free dot | `#ff453a` |
| error text | `#ffb340` |

## Sound

Two short, quiet cues, on by default, a setting to turn them off: a soft rising tick (25 ms)
when listening starts, a soft falling tick when the text is pasted. No sound on cancel or
error (the error text says enough). Played from memory, never decoded on the hot path.

## Focus and input rules (non-negotiable)

- The Bar window never becomes the foreground window and never takes keyboard focus
  (Windows: `WS_EX_NOACTIVATE`, shown with `SW_SHOWNOACTIVATE`).
- It ignores the mouse entirely: clicks pass through to whatever is under it
  (`WS_EX_TRANSPARENT`).
- No taskbar button, no Alt+Tab entry (`WS_EX_TOOLWINDOW`).
- It is created once at startup, hidden, with its page loaded: showing it is a native call
  plus a state change, never a page load.

## Timing budgets (measured, like everything else)

| From | To | Budget |
|---|---|---|
| key down | Bar's first visible frame | under 16 ms p50, 33 ms p99 |
| a level change | the waveform drawing it | one frame |
| a segment transcribed | its words visible | one frame |
| paste sent | Done check visible | one frame |

## Acceptance checks

Each is checked on the real machine, at 100% and 150% display scaling, light and dark Windows
themes, over a white page and a black terminal.

1. Pressing Right Ctrl shows the Bar within one frame; it never steals the focus (the caret in
   the target app keeps blinking; typing after the dictation goes to the target).
2. Clicking where the Bar is clicks the app under it.
3. The waveform moves with the voice and rests still at 3 px in silence; no stutter during
   transcription work.
4. Live words appear while speaking; words already shown never jump.
5. Right Ctrl + C copies and the Bar never appears.
6. Esc cancels with no check, no sound.
7. A long dictation keeps the newest words readable, oldest faded at the left.
8. Done shows the check briefly and hides; nothing lingers.
9. "Focus moved" shows when the target window closed or changed during the dictation.
10. Text is crisp at every scale; the pill's edge is anti-aliased, no seams or halos on white.
