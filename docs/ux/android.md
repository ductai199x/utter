# Utter on Android: the bubble

Dictation into any app on the phone, the way the Bar does it on the desktop: hold, speak, let
go, and the cleaned text is in the field. Status: **spec, not built**. Decisions and
measurements behind it: docs/plan-android.md. The implementation is reviewed against the checks
at the end.

## What it is for

1. Dictate into whatever text field has the focus, in any app, keeping the person's own
   keyboard (Gboard, SwiftKey).
2. Show at once that Utter is listening, then what it understood, without covering the field.
3. Be gone when no text field is focused.

## How it reaches other apps

An accessibility service (the only way an app can see the focused field and write into another
app without replacing the keyboard; Wispr Flow does the same). It shows the bubble as an
accessibility overlay, so no "draw over other apps" permission is needed. It reads only what it
needs: which field has the focus, whether it is a password field, and the text around the cursor
when inserting.

- **Never in sensitive fields**: password, PIN and credit-card fields (input type and autofill
  hints) hide the bubble; so does any field the app marks as sensitive.
- **Nothing leaves the phone**: audio, transcripts and text stay on the device, as on the desktop.

## The bubble

- A 48 dp circle with Utter's mark, at the right edge of the screen, vertically at 60% of the
  space above the keyboard. It appears when an editable field gains the focus and the keyboard
  is shown, and fades out (150 ms) when either goes.
- Draggable along the screen edges; it remembers its edge and height per app.
- Never takes the focus from the field and never moves the cursor.

## Dictating

| State | Shown when | Looks like | Leaves to |
|---|---|---|---|
| Idle | a text field is focused | the bubble, 70% opaque | Listening (press) |
| Listening | the bubble is held | the bubble grows to 56 dp, fully opaque, with a live ring that follows the voice level; a pill above the keyboard shows the words so far | Finishing (release), Cancelled (drag off) |
| Hands-free | after a double tap | as Listening, with a small red dot on the bubble; no need to hold | Finishing (tap), Cancelled |
| Finishing | release until the text is in the field | the ring settles to a slow sweep; the pill keeps the text, dimmed | Done, Error |
| Done | text inserted | a check on the bubble for 400 ms | Idle |
| Cancelled | dragged off the bubble before release | the pill fades at once, nothing inserted | Idle |
| Error | nothing heard, focus lost, insert blocked | one line in the pill, e.g. "Couldn't type here: copied instead" | Idle after 2.5 s |

- A press of less than 150 ms does nothing (a stray touch), as on the desktop.
- A short haptic tick when listening starts, another when the text lands. No sounds.
- The words in the pill are live (each pause is cleaned and shown, the words since are
  provisional and dimmed), exactly as the Bar.

## Inserting

1. Into the focused field at the cursor: the cleaned text replaces the selection, or goes in at
   the cursor with a space where one is needed, through the accessibility node's set-text
   action, keeping the cursor after the inserted text.
2. When that is refused (some apps' custom fields): paste through the clipboard, then put back
   what was on the clipboard.
3. When both fail, or the focus moved away: the text goes to the clipboard and History, and the
   pill says so.

## First run

One screen at a time, each with a single action:

1. **What Utter does**, in one sentence, and that nothing leaves the phone.
2. **Microphone**: the system permission prompt.
3. **Accessibility**: why it is needed (one paragraph, the same three points as above), then a
   button to the system's accessibility settings; Utter notices the service is on and moves on.
4. **Models**: the download (speech and cleanup, about 2 GB), with size and progress, on Wi-Fi
   unless the person chooses otherwise. Pausable; resumes after a restart.
5. **Try it**: a text field on this screen to dictate into, the bubble shown next to it.

## Performance (measured on the test phone, docs/plan-android.md)

- The speech model runs on the GPU (about 330 ms for 5 s of speech), the cleanup model on the
  CPU (about 1.5 s for a sentence). Both stay loaded while the service runs, so a dictation
  never waits for a load; the first load after a restart takes about 9 s and happens in the
  background.
- Release to text, target: 2 s for a sentence at p50, with the live words in the pill from the
  first pause.

## Checks

- [ ] The bubble appears within 200 ms of a text field gaining the focus with the keyboard up,
      and never in a password, PIN or card field.
- [ ] Holding shows Listening within 100 ms; a stray touch (under 150 ms) shows nothing.
- [ ] The text lands in the field at the cursor, with the right spacing, in Messages, Gmail,
      Chrome, Keep and Slack; when set-text is refused, the paste path works and the clipboard
      is restored.
- [ ] Dragging off the bubble cancels: nothing inserted, nothing kept.
- [ ] No sound is ever played; the microphone is open only while listening.
- [ ] Release to text p50 at or under 2 s on the test phone, measured in the app.
- [ ] The service survives the app being swiped away and the phone sleeping, and the bubble
      comes back on the next focused field.
