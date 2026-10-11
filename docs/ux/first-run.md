# First run

What a person sees the first time Utter starts, from the installer to their first dictation.
Status: **spec, not built**. It replaces `utter-cli setup` for everyone but developers.

## What it is for

1. Say early, and plainly, if this computer cannot run Utter, before downloading anything.
2. Get the 8 GB of models onto the machine without the person having to watch it.
3. End with a dictation that worked, so the person knows the key, the Bar and the result.

## When it shows

- When Utter starts and a required component (crates/utter-runtime/components.json, not
  optional) is not installed. Dictation does not start until they all are.
- It is a normal window, 640 x 520, centered, not resizable, with the OS title bar. Closing it
  quits Utter (nothing works without the models); downloads resume on the next start.
- Once everything is installed, later starts go straight to the tray.

## Steps

One step at a time, a row of step dots at the top (4), the step's content in the middle, the
buttons at the bottom right (Back as a text button, the main action filled).

### 1. Welcome and system check

Title "Welcome to Utter". Line: "Hold a key, speak, and clean text appears where you type.
Everything runs on this computer; nothing you say leaves it."

A checklist, each row with a check or a cross and a short value:
| Check | Passes when | Shows |
|---|---|---|
| Graphics card | an NVIDIA GPU answers (gpu::probe) | its name |
| GPU memory | at least 8 GB | "24 GB" |
| Graphics driver | supports CUDA 13.0 or newer | "CUDA 13.4" |
| Disk space | the data folder's drive has the remaining download plus 2 GB free | "120 GB free, 7.8 GB needed" |

All pass: **Continue**. One fails: the row says what to do in one line ("Update the NVIDIA
driver to version 580 or newer", "Free up 6 GB on C:"), and Continue is replaced by **Check
again**. No NVIDIA GPU: "Utter needs an NVIDIA graphics card (8 GB or more). Support for
other cards is planned." and **Quit**.

### 2. Microphone

Title "Your microphone". A dropdown of input devices (System default first) and a level meter
under it: 24 bars, moving with the voice. Line: "Say something. The bars should move."
Choosing a device saves it to settings.toml. **Continue**.

### 3. Download

Title "Downloading the models". Line: "About 7.8 GB. You can use your computer meanwhile;
Utter will let you know when it is ready."
- One progress bar for everything, with "3.1 of 7.8 GB" on the left and "about 4 min left" on
  the right (from the speed over the last 10 s; nothing until there are 3 s of data).
- Under it, what is being fetched now in muted text: "Speech model", "Cleanup model",
  "GPU runtime". After all downloads: "Checking files..." then "Getting ready..." while the
  models load.
- An error shows in amber under the bar with **Retry**: "Download interrupted. Retry picks up
  where it stopped." Retry resumes; nothing is downloaded twice.
- Done: it moves on by itself to step 4.

### 4. Try it

Title "Try it". Line: "Click in the box, hold **Right Ctrl**, and say:" then the sentence in
quotes: "Hi team, the meeting moved to three, no, four o'clock. Bring the slides."
A text box, 4 lines, focused. The Bar appears as it does everywhere. When text arrives in the
box: a check and "That's it. Hold Right Ctrl in any app to dictate. Double-tap it to keep
talking hands-free." **Finish** closes the window; Utter keeps running in the tray, and the
tray tooltip says "Utter: hold Right Ctrl to dictate".

The step can be skipped (**Skip** text button).

## Layout (logical pixels)

- Window 640 x 520, theme tokens of the Home window (light and dark follow the OS).
- Step dots: 4 dots, 6 px, 8 px apart, centered 28 px from the top. Done and current dots use
  the accent (the current one stretched to 18 px wide, a pill); the rest are the border color.
- Content column: 48 px side padding, starting 72 px from the top. Title 24/32 semibold, line
  14/20 muted under it, 8 px apart; the step's body 28 px below.
- Buttons: a bar 24 px from the bottom, right aligned, 8 px apart. The main action is filled
  with the accent, 36 px high, at least 112 px wide; Back and Skip are text buttons.

## States, step by step

| Step | State | Shows | Main action |
|---|---|---|---|
| 1 | checking | each row with a small spinner (the check takes under a second) | Continue, disabled |
| 1 | all pass | green check per row | Continue |
| 1 | one fails | amber cross, the fix line under that row | Check again |
| 1 | no GPU | one row "None found", the explanation in a panel | Quit |
| 2 | opening | the dropdown and a resting meter | Continue |
| 2 | listening | the meter moves | Continue |
| 2 | cannot open | amber line under the dropdown ("This microphone could not be opened. Choose another.") | Continue (the system default is used) |
| 3 | downloading | the bar fills; what is fetched; time left | none (it advances itself) |
| 3 | checking files | the bar full, "Checking files..." | none |
| 3 | loading | the bar full and shimmering, "Getting ready..." | none |
| 3 | error | amber line under the bar | Retry |
| 4 | waiting | the sentence, the focused box | Finish, disabled; Skip |
| 4 | text arrived | the check line | Finish |

- The download starts when step 3 first shows and runs on its own from then on. Back from
  step 3 is not offered.
- The meter's microphone is closed when step 2 is left, so the dictation pipeline can open it.

## Motion

- Steps: the outgoing content fades out over 120 ms; the incoming one fades in and slides
  8 px from the right over `--dur-enter` (Back slides from the left). Dots move with
  `--dur-resize`.
- The progress bar's width eases over 300 ms, so bursts of progress read as steady. While
  loading, a soft highlight sweeps across it every 1.6 s.
- The meter follows the level with a 30 ms attack and 140 ms release, like the Bar's waveform;
  resting bars are 3 px.
- With "reduce motion" set in the OS, nothing slides or sweeps: changes are instant fades.

## Keyboard

Enter presses the main action (except in the step 4 text box); Esc does nothing (closing quits
and should be deliberate). Tab reaches every control; focus rings as in the Home window. The
main action takes the focus when a step shows, except step 4, where the text box does.

## Acceptance checks

1. On a machine without the models, Utter opens First run, not the tray only.
2. With a too-old driver (simulated), the driver row fails with the update line, and nothing
   downloads.
3. The level meter moves with the voice within one frame.
4. The download survives a quit mid-way: on the next start it continues from where it stopped
   (the bar starts at the bytes already there).
5. Pulling the network cable shows the amber error; Retry after reconnecting continues.
6. The test dictation lands in the box, cleaned ("four o'clock", not "three, no, four").
7. After Finish, a restart goes straight to the tray.
