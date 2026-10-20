# First run

What a person sees the first time Utter starts, from the installer to their first dictation,
and what the same window does later for updates, repairs and start failures. Status for 0.1.11:
**built** (`app/src/setup`, `app/src-tauri/src/setup.rs`, `app/src-tauri/src/problem.rs`). It
replaces `utter-cli setup` for everyone but developers.

One window, `setup.html`, opened in a mode: `first`, `update`, `model`, `runtime`, `repair`,
`problem` or `try`. The core does the work and reports; the page shows it and sends intent.

## What it is for

1. Say early, and plainly, if this computer cannot run Utter, before downloading anything.
2. Get the models onto the machine without the person having to watch it.
3. End with a dictation that worked, so the person knows the key, the Bar and the result.

## When it shows

- When Utter starts and a required component (`crates/utter-runtime/components.json`) of the
  chosen model is not installed or not intact. Dictation does not start until they are all
  there, except in update and model mode (below).
- It is a normal window, 640 x 640 on Windows (640 x 720 on Linux and macOS, which also check
  keyboard access), centered, not resizable, with the OS title bar. Closing it quits Utter
  while dictation has not started (nothing works without the models); downloads resume on the
  next start. Once dictation runs (a model switch from Home), closing only closes.
- Once everything is installed, later starts go straight to the tray (with the splash,
  docs/ux/splash.md).

## Models on offer

Three models, by name in the UI (`utter_pipeline::settings::Model`):

| Name | Setting | What it is | Offered |
|---|---|---|---|
| Quality | `quality` | the bigger cleanup model (fine-tuned E4B) | with a graphics card that fits it |
| Light | `light` | the smaller one (fine-tuned E2B) | everywhere it runs |
| Extra light | `mini` | the smallest (a 1B, 720 MB) | only when running on the processor |

GPU memory each needs on CUDA (`gpu_need` in `crates/utter-pipeline/src/lib.rs`): Quality
4.8 GB, Light 3.25 GB, Extra light 2.0 GB. On Vulkan, Metal and the processor the figures are
6.2, 4.5 and 3.0 GB (the last three are estimates, not measured). A card must have the need
plus 1 GB (`card_need`). The prompted 26B is never offered: the fine-tuned E4B beats it on every
eval (docs/cleanup.md).

## Steps (first run)

One step at a time, a row of step dots at the top (5), the step's content in the middle, the
buttons at the bottom right (Back as a text button, the main action filled).

### 1. Welcome and system check

Title "Welcome to Utter" with the logo. Line: "Hold a key, speak, and clean text appears
where you type." Under the checklist, a "Private by design" panel (five lines: everything
happens on this computer, the internet is only for models and updates, no reading of files or
screen, history stays local and can be turned off, open source).

A checklist, each row with a spinner, check or cross and a short value:

| Check | Passes when | Shows |
|---|---|---|
| Graphics card (Mac: GPU) | an adapter answers; with none on Windows or Linux the row reads "Runs on: The processor" and passes, with "No graphics card found: Utter runs on the processor, with its smaller models." | its name |
| GPU memory (Mac and processor: Memory) | at least the card need of Light | "24 GB" |
| Graphics driver | NVIDIA: CUDA 13.0 or newer (the fix line says driver 580 or newer); Vulkan: present. Not shown on the processor or Mac | "CUDA 13.4" |
| Keyboard access (Linux, macOS) | the udev rule (Linux) or the macOS permissions are in place | "Allowed" |
| Disk space | the data folder's drive has the smallest download on offer plus 2 GB free | "186.2 GB free, 6.0 GB needed" |

All pass: **Continue**. One fails: the row says what to do in one line ("Update the NVIDIA
driver to version 580 or newer, then check again.", "Free up 6.0 GB on the drive with <data
folder>."), and Continue is replaced by **Check again**. Only keyboard access missing: the
button is **Allow** (a password dialog on Linux, System Settings on macOS); Utter checks again
and, on macOS, restarts into this step once the permission applies. No usable adapter at all:
"None found", the panel "Utter could not read this computer's graphics card or memory. It runs on an NVIDIA, AMD or Intel graphics card, or on the processor with 8 GB of memory or more." and **Quit**.

### 2. Choose your model

Title "Choose your model". Line: "Bigger models write more carefully. You can change this later
in Settings."

Cards side by side, one per model on offer:

| Card | Says |
|---|---|
| Quality | "The bigger model. Most accurate; best for most cards." (Mac: "The bigger model. Most accurate.") |
| Light | "The smaller model. Runs on 4 GB graphics cards." (processor: "...Runs on the processor."; Mac: "The smaller model. Uses less memory.") |
| Extra light | "The smallest model. Fastest on the processor." |

- Each card: the name, its sentence, "N GB GPU memory" ("N GB memory" on the processor and Mac)
  and "N GB download" (the bytes still missing; "Installed" when none).
- The best fit is selected when the step opens and marked **Recommended**: Extra light on the
  processor; otherwise Quality when the card has the memory for it, else Light.
- A model this machine cannot run is disabled, with "Needs a N GB graphics card" ("Needs a
  graphics card" on the processor; "Needs N GB of memory" on a Mac).
- Choosing a card saves it (settings.toml `model`) and checks the disk again against that
  model's download before moving on.
- Keyboard: arrow keys move between cards that fit, Enter continues.

### 3. Microphone

Title "Your microphone". A dropdown of input devices (System default first) and a level meter
under it: 64 bars over the last 1.5 s, moving with the voice. Line: "Say something. The bars
should move." Choosing a device saves it to settings.toml. **Continue**.

If nothing is heard after 6 s: "Nothing heard yet. Check that a microphone is plugged in and
not muted, or choose another."

### 4. Download

Title "Downloading the models". Line: "About N GB. You can use your computer meanwhile; Utter will
let you know when it is ready." (N is what the chosen model still needs.)
- One progress bar for everything (bytes of an earlier try count as done), with "3.1 of 6.0 GB"
  on the left and "about 4 min left" or "less than a minute left" on the right (from the speed
  over the last 10 s; nothing until there are 3 s of data).
- Under it, what is being fetched now in muted text: "Speech model", "Cleanup model", "GPU
  runtime" ("Processor runtime" on the processor). After all downloads: "Checking files..." then
  "Getting ready..." while the models load.
- An error shows in amber under the bar with **Retry** (messages under "Download errors").
  Retry resumes; nothing is downloaded twice.
- Footer note: "Closing this window pauses the download; it resumes next time."
- Done: it moves on by itself to step 5.

A first install is about 7 GB for Quality and 5 GB for Light on Windows with CUDA (README); the
card and the line above show the exact figure for the machine.

### 5. Try it

Title "Try it". Line: "Hold **Right Ctrl**, say this, then let go:" (the key the person
chose) then the sentence in a quote: "Hi team, the meeting moved to three, no, four o'clock.
Bring the slides." A text box, 3 lines, focused. The Bar appears as it does everywhere. If
dictation is still loading the page says "Getting ready...". When text arrives in the box: a
check, "You said "three, no, four", and Utter wrote "four"." (only when the text shows the
correction applied) and "Hold Right Ctrl in any app to dictate. Double-tap it to keep talking
hands-free." **Finish** (enabled once text arrived) closes the window; Utter keeps running in
the tray, and the tray tooltip says "Utter: hold Right Ctrl to dictate".

**Skip** (text button) closes it too. Finish and Skip both record that Try it is done
(`tried` marker in the data folder); a first dictation anywhere also records it.

## Layout (logical pixels)

- Window 640 wide, theme tokens of the Home window (light and dark follow the OS).
- Step dots: 6 px, 8 px apart, centered 28 px from the top, shown when there are two or more
  steps. Done and current dots use the accent (the current one stretched to 18 px wide, a
  pill); the rest are the border color.
- Content column: 48 px side padding. Title 24/32 semibold, line muted under it.
- Buttons: a bar 24 px from the bottom, right aligned, 8 px apart. The main action is filled
  with the accent; Back, Skip, Open logs and Quit are text buttons.

## States, step by step

| Step | State | Shows | Main action |
|---|---|---|---|
| 1 | checking | each row with a small spinner | Continue, disabled |
| 1 | all pass | green check per row | Continue |
| 1 | one fails | cross, the fix line under that row | Check again |
| 1 | only keyboard access missing | the fix line | Allow |
| 1 | no adapter | one row "None found", the explanation in a panel | Quit |
| 2 | choosing | cards, the best fit selected | Continue |
| 2 | disk too small for the choice | amber line with the fix | Continue stays |
| 3 | opening | the dropdown and a resting meter | Continue |
| 3 | listening | the meter moves | Continue |
| 3 | cannot open | amber line under the dropdown ("This microphone could not be opened. Choose another."; with none connected: "No microphone found. Plug in a microphone or headset; you can continue meanwhile.") | Continue (the system default is used) |
| 4 | downloading | the bar fills; what is fetched; time left | none (it advances itself) |
| 4 | checking files | the bar full, "Checking files..." | none |
| 4 | loading | the bar full and shimmering, "Getting ready..." | none |
| 4 | error | amber line under the bar | Retry |
| 5 | waiting | the sentence, the focused box | Finish, disabled; Skip |
| 5 | text arrived | the check lines | Finish |

- The download starts when step 4 first shows and runs on its own from then on. Back is not
  offered on step 4.
- The meter's microphone is closed when step 3 is left, so the dictation pipeline can open it.

## Motion

- Steps: the outgoing content fades out over 120 ms; the incoming one fades in and slides
  8 px from the right (Back slides from the left) over 120 ms. Dots move with
  `--dur-resize`.
- The progress bar's width eases over 300 ms, so bursts of progress read as steady. While
  loading, a soft highlight sweeps across it every 1.6 s.
- The meter follows the level with a 30 ms attack and 140 ms release, like the Bar's waveform
  (`app/src/lib/waveform.ts`); resting bars are 3 px.
- With "reduce motion" set in the OS, nothing slides: changes are instant fades.

## Keyboard

Enter presses the main action (except in the Try it text box or on a focused button); Esc does
nothing (closing quits and should be deliberate). Tab reaches every control; focus rings as in
the Home window. The main action takes the focus when a step shows, except Try it, where the
text box does.

## When Utter starts: the paths

At every start, before anything else, Utter finishes a repair that was waiting for a restart,
removes component versions that components.json no longer lists once their replacement is
intact (`utter_runtime::prune`: an update leaves the old model behind, 5 GB; until the new one
is whole, the old one is what dictation runs on), then checks each required component:
installed, and every file its receipt lists still there at its size (a quick check, no
hashing). Then (`setup::mode_for`, `main.rs`):

| Situation | What shows |
|---|---|
| Everything is installed and intact, Try it done | nothing but the splash (not at sign-in): dictation starts, the tray says it is ready |
| Everything is installed and intact, Try it never finished | dictation starts and the window opens in **try mode**: Try it alone |
| First run has not finished: nothing installed yet, or it was interrupted (no `setup-complete` file in the data folder, written once everything is installed) | First run, as above; the download resumes where it stopped |
| A component has a new version (components.json changed with an update; the old version is still here) | dictation starts on the old version, and the window opens in **update mode**: the system check titled "Utter needs to download an update" ("N GB to download. Dictation keeps working on the current version until it is done."), then the download; the new version then loads beside the old, takes over, and the window shows "Utter is ready" and closes itself. No model, microphone or Try it steps. |
| A component is damaged, or gone with nothing it replaces | the same, titled "Some of Utter's files are missing or damaged" (**repair mode**); dictation does not start until it is done |
| The model chosen in Settings is not downloaded yet, another one is | dictation starts on the other model at once, and the window opens in **model mode**: "Utter needs to download the model you chose" ("Dictation keeps the current model until it is done."); Utter switches when it is done. The same as choosing it while Utter runs. |
| The runtime chosen in Settings (CUDA, Vulkan, processor) has components that were never here | **runtime mode**: "Utter needs to download a few files" ("Dictation starts again as soon as it is done."); nothing runs until they are here |
| Dictation could not start | the same window in **problem mode** (below) |

When an update, model or repair download ends, the window shows "Utter is ready. Hold Right
Ctrl in any app to dictate." for 1.5 s and closes. Closing an update or model window keeps the
download going (the footer says so).

## Problem mode: when dictation cannot start

One screen: an amber icon, a title that names the cause, one or two lines saying what to do,
and the actions that fit. Never a raw error alone; the technical detail is under "Details"
(collapsed), with **Open logs** where it helps. It opens before the splash closes, so there is
no blank moment.

| Cause (how it is recognized) | Title | What to do | Actions |
|---|---|---|---|
| GPU memory full: an out-of-memory error, or less free GPU memory than the tier needs | Not enough free GPU memory | "Utter needs about 4.8 GB free on the graphics card; 2.1 GB are free now. Close games or other apps using the GPU, then retry." On Quality with Light on offer, it adds "Or choose the Light model in Settings: it needs about N GB." | Retry, Quit |
| No microphone at all | No microphone found | "Plug in a microphone or headset, then retry." | Retry, Quit |
| Keyboard access missing (Linux, macOS) | Utter cannot use the keyboard | the Allow text from step 1 | Allow, Open logs, Quit |
| A model file will not load (missing, damaged), or the GPU libraries fail (a cuDNN failure, a library that will not load) | Some of Utter's files are damaged | "Repair checks every file and downloads again the ones that are damaged." | Repair, Quit |
| The GPU driver fails to start the models | The graphics driver could not run Utter | CUDA: "Update the NVIDIA driver, restart the computer, then retry."; Vulkan: "Update the graphics driver, ..."; Mac: "Update macOS, ..."; processor: "Restart the computer, then retry." | Retry, Open logs, Quit |
| Anything else | Utter could not start | the error in one line | Retry, Open logs, Quit |
| A model switch failed after its download (Home) | Not enough free GPU memory for this model, or The new model could not start | "The previous model is still in use. Retry, or open the logs." | Retry, Open logs, Close |

Repair hashes every file with a checksum on its own thread; when every file checks out, the
GPU libraries (extracted from archives, without per-file checksums) are fetched again. After a
failed start those libraries are still loaded and cannot be removed: Utter writes down which
(`repair-pending`), restarts, removes them before anything loads, and the download starts by
itself (the person already asked for it). One click, then about 30 s for the libraries
(timing from the old spec; not re-measured).

A saved microphone that is not connected is not a failure: dictation uses the system default,
and the Home window's microphone row says the saved one is not connected.

## Download errors

| Cause | Message under the bar | Action |
|---|---|---|
| The disk filled up | "The disk is full (N GB free on C:). Free up some space, then retry." | Retry |
| No connection | "Can't reach the download server. Check your internet connection, then retry." | Retry |
| The server refused (an HTTP error) | "The download server answered {status}. Try again in a few minutes." | Retry |
| A file arrived damaged (size or checksum) | "A file arrived damaged. Retry downloads it again." | Retry |
| Anything else | "Download interrupted ({detail}). Retry picks up where it stopped." | Retry |

## Acceptance checks

"Met" needs evidence in code, tests or measurements; "unverified" means none of those shows
it; the checks were run by hand on earlier versions, but no record is kept in the repo.

| # | Check | Status |
|---|---|---|
| 1 | On a machine without the models, Utter opens First run, not the tray only | Met in code (`mode_for`, `setup-complete` marker) |
| 2 | With a too-old driver (simulated), the driver row fails with the update line, and nothing downloads | Met in code (Continue is replaced by Check again; the download starts only from step 4); the `?fail=driver` stand-in plays it in a browser |
| 3 | The level meter moves with the voice within one frame | Level sent every 16 ms, drawn each frame; unverified on screen |
| 4 | The download survives a quit mid-way: on the next start it continues from where it stopped (the bar starts at the bytes already there) | Met: test `an_interrupted_download_resumes_where_it_stopped` (`crates/utter-runtime`), and `download` counts finished bytes as done |
| 5 | Pulling the network cable shows the amber error; Retry after reconnecting continues | Message mapping met (`download_failures_name_their_cause` test); live check unverified |
| 6 | The test dictation lands in the box, cleaned ("four o'clock", not "three, no, four") | Unverified here; the cleanup evals are in docs/cleanup.md |
| 7 | After Finish, a restart goes straight to the tray | Met in code (`tried` and `setup-complete` markers) |
| 8 | Deleting a model file and restarting shows "Some of Utter's files are missing or damaged"; Repair downloads only that component again, and dictation starts | Met in code (`mode_for`, `setup_repair`); unverified on the current build |
| 9 | Starting while a game holds most of the GPU memory shows "Not enough free GPU memory" with the free amount; Retry after closing the game starts dictation | Met in code and by test (`gpu_out_of_memory_is_recognized_from_onnxruntime_text`); live check unverified |
| 10 | A components.json update (a new cleanup model) shows update mode while dictation keeps working on the old model, downloads only the new component, switches to it, removes the old one's folder at the next start, and needs no other step | Met in code (`Mode::Update`, `prune`); unverified on the current build |
| 11 | With the saved microphone unplugged, dictation starts on the default one | Met in code (`check_microphone` in `crates/utter-pipeline/src/lib.rs`); unverified on the current build |
