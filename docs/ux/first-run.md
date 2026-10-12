# First run

What a person sees the first time Utter starts, from the installer to their first dictation.
Status: **built** (app/src/setup). It replaces `utter-cli setup` for everyone but developers.

## What it is for

1. Say early, and plainly, if this computer cannot run Utter, before downloading anything.
2. Get the 4 to 6 GB of models onto the machine without the person having to watch it.
3. End with a dictation that worked, so the person knows the key, the Bar and the result.

## When it shows

- When Utter starts and a required component (crates/utter-runtime/components.json, not
  optional) is not installed. Dictation does not start until they all are.
- It is a normal window, 640 x 520, centered, not resizable, with the OS title bar. Closing it
  quits Utter (nothing works without the models); downloads resume on the next start.
- Once everything is installed, later starts go straight to the tray.

## Steps

One step at a time, a row of step dots at the top (5), the step's content in the middle, the
buttons at the bottom right (Back as a text button, the main action filled).

### 1. Welcome and system check

Title "Welcome to Utter". Line: "Hold a key, speak, and clean text appears where you type.
Everything runs on this computer; nothing you say leaves it."

A checklist, each row with a check or a cross and a short value:
| Check | Passes when | Shows |
|---|---|---|
| Graphics card | an NVIDIA GPU answers (gpu::probe) | its name |
| GPU memory | enough for the smallest model on offer (utter_pipeline::card_need: 4 GB with Light) | "24 GB" |
| Graphics driver | supports CUDA 13.0 or newer | "CUDA 13.4" |
| Disk space | the data folder's drive has the remaining download plus 2 GB free | "120 GB free, 6.0 GB needed" |

All pass: **Continue**. One fails: the row says what to do in one line ("Update the NVIDIA
driver to version 580 or newer", "Free up 6 GB on C:"), and Continue is replaced by **Check
again**. No NVIDIA GPU: "Utter needs an NVIDIA graphics card. Support for other cards is
planned." and **Quit**.

### 2. Choose your model

Title "Choose your model". Line: "Bigger models write more carefully. You can change this later
in Settings."

Two cards side by side, the two models (Tai, 2026-10-11; Light is listed once its model is
published):

| Card | Says | Needs |
|---|---|---|
| Quality | "The bigger model. Most accurate; best for most cards." | about 4.8 GB of GPU memory with speech, on cards of 6 GB or more (the fine-tuned E4B) |
| Light | "The smaller model. Runs on 4 GB graphics cards." | about 3.25 GB of GPU memory with speech, on 4 GB cards (the fine-tuned E2B) |

- Each card: the name, its sentence, "N GB GPU memory", and "N GB download".
- The best fit for this GPU is selected when the step opens and marked **Recommended** (a
  badge on the card's top edge): Quality when the card is big enough for it (utter_pipeline::card_need,
  about 5.8 GB), Light below.
- A model this GPU cannot run is shown disabled, with "Needs an N GB graphics card".
- The prompted 26B is never offered: the fine-tuned E4B beats it on every eval.
- Choosing a card saves it (settings.toml `model`); the download step fetches what it needs,
  and the disk check of step 1 is redone for it before the download starts.
- Keyboard: arrow keys move between cards, Enter continues.

### 3. Microphone

Title "Your microphone". A dropdown of input devices (System default first) and a level meter
under it: 24 bars, moving with the voice. Line: "Say something. The bars should move."
Choosing a device saves it to settings.toml. **Continue**.

### 4. Download

Title "Downloading the models". Line: "About N GB (what the chosen model still needs). You can use your computer meanwhile;
Utter will let you know when it is ready."
- One progress bar for everything, with "3.1 of 6.0 GB" on the left and "about 4 min left" on
  the right (from the speed over the last 10 s; nothing until there are 3 s of data).
- Under it, what is being fetched now in muted text: "Speech model", "Cleanup model",
  "GPU runtime". After all downloads: "Checking files..." then "Getting ready..." while the
  models load.
- An error shows in amber under the bar with **Retry**: "Download interrupted. Retry picks up
  where it stopped." Retry resumes; nothing is downloaded twice.
- Done: it moves on by itself to step 5.

### 5. Try it

Title "Try it". Line: "Click in the box, hold **Right Ctrl**, and say:" then the sentence in
quotes: "Hi team, the meeting moved to three, no, four o'clock. Bring the slides."
A text box, 4 lines, focused. The Bar appears as it does everywhere. When text arrives in the
box: a check and "That's it. Hold Right Ctrl in any app to dictate. Double-tap it to keep
talking hands-free." **Finish** closes the window; Utter keeps running in the tray, and the
tray tooltip says "Utter: hold Right Ctrl to dictate".

The step can be skipped (**Skip** text button).

## Layout (logical pixels)

- Window 640 x 520, theme tokens of the Home window (light and dark follow the OS).
- Step dots: 5 dots, 6 px, 8 px apart, centered 28 px from the top. Done and current dots use
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

## When Utter starts: three paths

At every start, before anything else, Utter removes component versions that components.json
no longer lists (an update leaves the old model behind: 5 GB), then checks each required
component: installed, and every file its receipt lists still there at its size (a quick check,
no hashing). Then:

| Situation | What shows |
|---|---|
| Everything is installed and intact | nothing: dictation starts, the tray says it is ready |
| First run has not finished: nothing installed yet, or it was interrupted (no `setup-complete` file in the data folder, written once everything is installed) | First run, as above; the download resumes where it stopped |
| A component has a new version (components.json changed with an update; the old version is still here) | the same window in **update mode**: the system check titled "Utter needs to download an update", then the download, then it closes itself once dictation is ready. No microphone or Try it steps. |
| A component is damaged, or gone with nothing it replaces | the same, titled "Some of Utter's files are missing or damaged" (**repair mode**) |
| The model chosen in Settings is not downloaded yet, the other one is | dictation starts on the other model at once, and the window opens in **model mode**: "Utter needs to download the model you chose"; Utter switches when it is done. The same as choosing it while Utter runs. |
| Dictation could not start | the same window in **problem mode** (below) |

## Problem mode: when dictation cannot start

One screen: an amber icon, a title that names the cause, one or two lines saying what to do,
and the actions that fit. Never a raw error alone; the technical detail is under "Details"
(collapsed), with **Open logs**.

| Cause (how it is recognized) | Title | What to do | Actions |
|---|---|---|---|
| GPU memory full: an out-of-memory error, or less free GPU memory than Utter needs | Not enough free GPU memory | "Utter needs about 6 GB free on the graphics card; 2.1 GB are free now. Close games or other apps using the GPU, then retry." | Retry, Quit |
| No microphone at all | No microphone found | "Plug in a microphone or headset, then retry." | Retry, Quit |
| A model file will not load (missing, damaged), or the GPU libraries fail (a cuDNN failure, a library that will not load) | Some of Utter's files are damaged | "Repair checks every file and downloads again the ones that are damaged." | Repair, Quit |
| The GPU driver fails to start the models | The graphics driver could not run Utter | "Update the NVIDIA driver, restart the computer, then retry." | Retry, Quit |
| Anything else | Utter could not start | the error in one line | Retry, Open logs, Quit |

Repair hashes every file with a checksum on its own thread; when every file checks out, the
GPU libraries (extracted from archives, without per-file checksums) are fetched again. After a
failed start those libraries are still loaded and cannot be removed: Utter writes down which,
restarts, removes them before anything loads, and the download starts by itself (the person
already asked for it). One click, then about 30 s for the libraries.

A saved microphone that is not connected is not a failure: dictation uses the system default,
and the Home window's microphone row says the saved one is not connected.

## Download errors

| Cause | Message under the bar | Action |
|---|---|---|
| The disk filled up | "The disk is full. Free up {n} GB on {drive}, then retry." | Retry |
| No connection | "Can't reach the download server. Check your internet connection, then retry." | Retry |
| The server refused (an HTTP error) | "The download server answered {status}. Try again in a few minutes." | Retry |
| A file arrived damaged (size or checksum) | "A file arrived damaged. Retry downloads it again." | Retry |
| Anything else | "Download interrupted ({detail}). Retry picks up where it stopped." | Retry |

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
8. Deleting a model file and restarting shows "Some of Utter's files are damaged"; Repair
   downloads only that component again, and dictation starts.
9. Starting while a game holds most of the GPU memory shows "Not enough free GPU memory" with
   the free amount; Retry after closing the game starts dictation.
10. A components.json update (a new cleanup model) shows update mode, downloads only the new
    component, removes the old one's folder, and needs no other step.
11. With the saved microphone unplugged, dictation starts on the default one.
