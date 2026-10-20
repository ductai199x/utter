# Utter: design

Utter is voice dictation that runs entirely on your own machine. Hold a key, speak, let go:
clean, well-punctuated text appears in whatever app has the cursor. It is a local answer to
Wispr Flow, built to feel instant.

This file is the source of truth for scope, architecture and decisions. A change of
architecture or scope changes this file in the same commit. The docs it links to are indexed in
[docs/README.md](docs/README.md).

**Status (0.1.10, 2026-10-20).** Released and in use:

| | Ships |
|---|---|
| Operating systems | Windows 10 and 11 (64-bit); Linux on GNOME and KDE, Wayland and X11 (Ubuntu 24.04+, Debian 13, Mint 22 from the `.deb`; Fedora 40+ and others from the AppImage); macOS 14+ on Apple Silicon |
| Backends | NVIDIA through CUDA; AMD and Intel through Vulkan (preview, not yet measured on a real AMD or Intel card); Apple's GPU through Metal; the processor alone |
| Models | Quality (Gemma 4 E4B fine-tuned, the default with a GPU), Light (Gemma 4 E2B fine-tuned), Extra light (Gemma 3 1B fine-tuned, for the processor); speech is parakeet-ultra on every tier |
| Language | English |
| Install | Windows: per-user NSIS installer. Linux: `.deb` and AppImage. macOS: a Homebrew cask whose tap is this repository (`Casks/utter.rb`) |
| Updates | Models and runtime: checksummed components, updated while dictation keeps running. The app: a daily check against GitHub, offered in the tray and Settings, installed only when the person clicks Update (from 0.1.10) |

Next: Android ([docs/android.md](docs/android.md)), then context from the text around the
cursor, styles per app, Command Mode and Vietnamese (section 2).

---

## 1. Principles

1. **Local only.** No audio, text, context or telemetry leaves the machine. The network is
   used only to download models and runtime components and to check for and download app
   updates. Both are shown to the person, and the update check can be turned off.
2. **Latency is the product.** Every stage has a budget (section 5). Every stage is measured
   on every dictation. Nothing counts as faster or slower until it is measured, and a change
   that costs latency for a small gain in quality is not made.
3. **Never lose words.** Audio capture starts before anything else. A failure at any stage
   still inserts the best text Utter has, or leaves it on the clipboard and in History.
4. **Never get in the way.** Utter never takes focus from the app being typed into. It always
   puts the person's clipboard back. It never types into the wrong window.
5. **Standalone.** Utter uses nothing installed on the machine but the GPU driver. Everything
   it runs on is a pinned, checksummed component (`crates/utter-runtime/components.json`).
6. **No slop.** A feature is done when it meets its latency budget, its eval numbers and its
   UX acceptance checks (section 12). "It works" is the starting point, not the finish.

---

## 2. Scope

One codebase for Windows, Linux and macOS (Rust core, Tauri 2 shell, Svelte 5 pages). Android
is a separate Kotlin app around the same Rust core.

### 2.1 Built (0.1.10)

| Feature | What it does | Where |
|---|---|---|
| Dictation | Hold the key to talk, double-tap it for hands-free (tap again to finish), Esc to cancel. Segments are transcribed while the person speaks. | 4, `crates/utter-pipeline` |
| Live text in the Bar | The words as they are heard, and at each pause the cleaned text so far; the paste is exactly the last cleaned text. | 4.2, [docs/ux/bar.md](docs/ux/bar.md) |
| Cleanup | Fillers, self-corrections ("2, no, 3"), stutters, spoken lists to bullets (a setting), spoken punctuation, numbers as typed, punctuation, casing and grammar. Every dictation is cleaned. | 6.2, [docs/cleanup.md](docs/cleanup.md) |
| Typed, never answered | Dictated questions and instructions are typed, not obeyed. A code check types the transcript if the output strays; a reply that runs away is retried once with a guard. | 4 step 7, `crates/utter-llm/src/cleanup.rs` |
| Dictionary | Up to 2,000 names and terms, added by hand or imported from a `.txt` or `.csv` file. They steer recognition (phrase boosting) and cleanup (only the terms heard in a dictation are sent). | 6.1, [docs/ux/home.md](docs/ux/home.md) |
| Insertion | Paste into the window focused at key down, clipboard restored; per-app paste rules (Ctrl+V, Ctrl+Shift+V, Shift+Insert, or typed keys); a space added when a dictation continues text; pastes confirmed where the app reports its text; a failed paste left on the clipboard with a one-click rule fix. | 7, [docs/ux/insertion.md](docs/ux/insertion.md) |
| Paste or copy the last dictation again, Retry last dictation | Shortcuts (4.1) and the tray menu. | 4.1 |
| History | Every dictation (typed text, what was heard, app, status), searchable and filterable, favorites, delete with undo, cleanup's changes marked word by word, retention (forever, 30 days, 24 hours, off). No audio. | 11, [docs/ux/history.md](docs/ux/history.md) |
| Home window | History, Dictionary and Settings (dictation key, microphone, model, what it runs on, processor threads, lists, history, logs, start at login, updates, paste rules). | 8, [docs/ux/home.md](docs/ux/home.md) |
| First run | System check, model choice with a recommendation for the machine, keyboard access (Linux, macOS), microphone level check, resumable download, a test dictation. Also the Update, Repair and Problem screens. | 8, [docs/ux/first-run.md](docs/ux/first-run.md) |
| Self-healing | The microphone reopens when it dies or the default changes; the cleanup server restarts when it exits; a damaged component is repaired in one click. | 4, 3.2 |
| App updates | Daily check of the latest release's `latest.json` (a setting, on by default); the person clicks Update; the download is checked against the updater key in the app. | 10, `app/src-tauri/src/update.rs` |

### 2.2 Next, in order

1. **Android** on the test phone: the Kotlin app with the bubble, the shared Rust core
   (`crates/utter-mobile`), the encoder on LiteRT, Extra light on the CPU. In progress
   ([docs/android.md](docs/android.md)).
2. **Long dictations:** cleanup that writes only its edits, so a long dictation is never
   re-cleaned whole (a 500-word window was tried and reverted on 2026-10-19 as too slow;
   [docs/cleanup.md](docs/cleanup.md)).
3. **AMD and Intel measured** on real cards, and the clock hold on Vulkan; the Vulkan backend
   leaves preview when it meets the budgets ([docs/platforms.md](docs/platforms.md)).
4. **Context:** the text around the cursor and the app given to cleanup, to spell names and
   match the field. Today the text before the cursor is read only for spacing and to confirm
   the paste; nothing of it reaches the model.
5. **Styles per app** (Formal, Neutral, Casual) and **Command Mode** (speak an instruction to
   rewrite the selection, or get an answer inserted). Neither exists in the code yet.
6. **Vietnamese**, with switching between the two languages mid-sentence. The candidates were
   measured on 2026-10-09 (Qwen3-ASR-1.7B and Cohere Transcribe tie); nothing is built.

### 2.3 Not built, no plan yet

Planned in the 2026-10-09 draft and not started: snippets, whisper mode (noise suppression,
gain, a lower voice threshold), the notetaker (system audio, speakers, summaries, an MCP
server), IDE file tagging, dictionary learning from the person's corrections, Stats, sound
cues, moving and docking the Bar, pausing media while dictating, audio playback in History,
more than one key per action and mouse buttons as keys. They stay possible; none is scheduled.

### 2.4 Out of scope

Teams, admin and enterprise features, cloud sync, accounts, telemetry.

---

## 3. Architecture

### 3.1 Processes

```
┌──────────────────────────── utter (one Tauri process) ────────────────────────────┐
│                                                                                    │
│  WebView pages (Svelte 5)            Rust core (crates/, in-process)               │
│  ┌───────────┐ ┌──────────────┐      ┌────────┐  ┌───────┐  ┌──────────┐            │
│  │ Bar       │ │ Home, Setup, │◄────►│pipeline│◄─┤ audio │◄─┤ hotkey   │ OS layer   │
│  │ (overlay) │ │ Splash, Rules│ IPC  │ (loop, │  └───────┘  └──────────┘            │
│  └───────────┘ └──────────────┘      │session)│──► asr (onnxruntime: CUDA, WebGPU,  │
│                                      │        │        or the CPU; Silero VAD)      │
│                                      │        │──► focus, caret (UIA / AT-SPI / AX) │
│                                      │        │──► insert (clipboard + paste keys)  │
│                                      │        │──► history.jsonl, settings.toml     │
│                                      └───┬────┘                                     │
└──────────────────────────────────────────┼─────────────────────────────────────────┘
                                           │ HTTP on 127.0.0.1, OpenAI-compatible API
                                  ┌────────▼────────┐
                                  │  llama-server   │  child process, kept warm,
                                  │  (llama.cpp)    │  restarted by the core if it dies
                                  └─────────────────┘
```

- **Rust core in the Tauri process.** Audio, the hotkey, speech recognition, insertion and
  storage share one process: no IPC hop and no sidecar start-up on the hot path.
- **llama-server is a separate process.** The LLM is the largest and least stable component.
  As a child process, a crash cannot take the app down, a model switch is a restart, and
  llama.cpp's official builds exist for every backend. The cleanup worker supervises it: a
  health check every 5 s while idle and after any failed request, restarts with waits of 1, 5,
  30 then 60 s, and a warm-up request. It ends when Utter ends (a job object on Windows,
  `PR_SET_PDEATHSIG` on Linux, a watcher on macOS).
- **The UI only renders.** Pages show state and send intent. All logic, timing and decisions
  stay in Rust, so they can be tested without a window.

### 3.2 Crates and packages

| Crate | Job |
|---|---|
| `utter-core` | Where data lives, the rotating logs (`utter.log`, `llama-server.log`), tracing setup |
| `utter-runtime` | The pinned components: download (resumable), SHA-256 check, install, repair, updates that keep the old version until the new one is whole; the backend (CUDA, Vulkan, Metal, CPU); the GPU clock hold |
| `utter-audio` | Microphone capture (`cpal`), a lock-free ring, the last 10 s of audio kept, resampling to 16 kHz, levels for the waveform, death and default-device detection |
| `utter-asr` | Parakeet TDT on onnxruntime (and on LiteRT for Android), phrase boosting, Silero VAD and the segmenter that cuts at pauses |
| `utter-llm` | llama-server as a child process, the cleanup request, the guards (`strayed`, the retry on a runaway reply, `tidy_output`); prompts in `crates/utter-llm/prompts/` |
| `utter-platform` | Every OS call, one module per OS behind the same functions: hotkey, shortcuts, insert, focus, caret, overlay, GPU and CPU, memory, disk, autostart, processes |
| `utter-pipeline` | The dictation loop and `Session` (shared with Android), the gate, live cleanup, settings, History, the dictation log, the runtime and model choice, GPU memory needs |
| `utter-mobile` | Android's core behind JNI: the session fed by the app's audio and bubble, events polled by Kotlin |
| `utter-cli` | Developer tools: `setup`, `doctor`, `dictate`, `bench`, `e2e-bench`, `llm-bench`, `vad` |
| `app/src-tauri` | The shell: windows, tray, IPC commands, first run, splash, problem screens, the updater |
| `android/` | The Kotlin app and its Gradle build; `android/native/` builds the core and llama-server per ABI |

Python lives in `tools/` only (`utter-eval`, `utter-train`, `utter-data`, `utter-bench`,
`utter-speech`, managed with uv). It is never shipped.

---

## 4. A dictation, step by step

```
idle ──key down──► listening ──key up──► finishing ──► cleaning ──► inserting ──► idle
                      │                                    │
                      │ Esc / another key / held < 150 ms  └─ cleanup down or timed out:
                      ▼                                       the transcript goes in
                   cancelled
```

1. **Key down.** The listener thread records the time and signals the pipeline; nothing else
   happens on that thread. The microphone is already open and keeps its last 10 s, so the
   dictation starts 300 ms before the key went down and the first word is never cut off. The
   Bar, created hidden with its page loaded, shows. On NVIDIA, one GPU thread starts napping in
   a tiny kernel so the clocks stay at P2 until key up: an idle GPU drops its clocks, a cold
   pass takes about 5 times as long, and cleanup is bound by memory bandwidth (2026-10-10,
   [docs/evals.md](docs/evals.md)). Keep-alive passes run while the person speaks, except on
   the processor, where they would take cores from real work.
2. **Focus capture.** On key down, Utter records the focused window and its app, and asks for
   the text before the cursor on a thread of its own. Insertion goes to that window and
   nowhere else.
3. **Streaming transcription while the key is held.** Silero marks pauses. Each finished
   segment goes to Parakeet at once, so at key up only the last segment is left. A cut needs
   384 ms of silence, 128 ms once a segment is past 8 s; a segment is at most 30 s (the
   encoder's limit is 41 s), cut at its quietest point.
4. **Live text.** The Bar shows the words as recognized, and at each pause the dictation so
   far is sent for cleanup and the cleaned text replaces them (section 4.2). A newer request
   makes queued older ones stale.
5. **Cleanup gate.** `gate::reason` names what in a transcript needs the model (a filler, a
   correction cue, a number, a dictionary term...). Since 2026-10-10 every dictation is cleaned
   anyway (`always_clean`); the gate's verdict is logged, and `e2e-bench --gated` measures the
   old behavior.
6. **Cleanup at key up.** If nothing new was heard since the last pause, the final cleanup is
   already done and the paste does not wait for it. The request is the fine-tuned prompt
   (`crates/utter-llm/prompts/cleanup-ft.md`), then `Lists: off` when lists are off, the
   dictionary terms heard in this dictation, and the transcript. llama-server runs with
   `--swa-full`, n-gram drafting from the prompt and `-ub 128` (section 6.2).
7. **Guards.** The reply limit is about the input's length. A reply that runs to the limit is
   retried once with a grammar that forbids a leading list marker and a mild repetition
   penalty. If cleanup fails, times out (3 s), the server is down, or the output strayed from
   the dictation (most of its content words new), the transcript goes in instead.
8. **Insertion.** Only if the window captured at key down still has the focus and the OS would
   deliver keys to it: save the clipboard, set the text, press the app's paste keys, restore
   the clipboard once the app has read it and only if nothing else changed it. Where the app
   reports its text, Utter reads it again (up to 400 ms) to confirm the paste landed. Otherwise
   the text stays on the clipboard and the Bar says so.
9. **Record.** History gets the typed text, the transcript, the app and the status. The
   dictation log (`<data dir>/dictations/`, development builds or the `debug_log` setting)
   keeps every recognizer pass, every Bar state, the cleanup request, the timings and the audio.

### 4.1 Shortcuts

| Action | Windows and Linux | macOS | Wispr Flow's Windows default (2026-10-09) |
|---|---|---|---|
| Push-to-talk (hold) | Right Ctrl | Right Option | Ctrl + Win |
| Hands-free (start, then stop) | Double-tap the key, tap again | the same | Ctrl + Win + Space |
| Cancel | Esc | Esc | Esc |
| Paste the last dictation again | Alt + Shift + Z | Ctrl + Cmd + V | Shift + Alt + Z |
| Copy the last dictation | Alt + Shift + X | Ctrl + Cmd + C | not surveyed |
| Command Mode (hold) | not built | not built | Ctrl + Win + Alt |

The dictation key is one key, chosen in Settings among keys that do nothing on their own: Right
Ctrl, Left Ctrl, Right Shift, Scroll Lock, F13-F24, and on a Mac Right Option and Right Command
(no Scroll Lock or F20-F24 there). Alt, Caps Lock, Insert and the mouse's side buttons are not
offered: they act when pressed alone. Utter only listens; the key still reaches the app.

Right Ctrl is half of ordinary shortcuts (Right Ctrl + C), so another key pressed while it is
held cancels the dictation without a trace and goes through to the app. A hold shorter than
150 ms is a tap and records nothing; a second press within 350 ms of a tap starts hands-free.
Injected key events (Utter's own Ctrl+V, other software) never act.

### 4.2 Live text: in the Bar, not in the field

Text is inserted into the app **once**, at the end, as Wispr Flow does. Typing into the app
while the person speaks, then revising it, fights the app (autocomplete, chat apps that send on
Enter, the undo history) and makes Ctrl+Z undo one fragment instead of the dictation.

What makes it feel live is the Bar: the cleaned text so far, with the words heard since the
last cleanup after it, dimmed as provisional. The paste is exactly the last cleaned text shown.

---

## 5. Latency budget

Budgets are p50 / p99. Measurements are the newest for each row, with their date and setup;
superseded runs are in [docs/evals.md](docs/evals.md). The development machine is an RTX 3090
Ti with a Ryzen 9 9900X on Windows 11.

| Step | Budget | Measured | Date, setup |
|---|---|---|---|
| Key down → Bar visible | one frame | not measured as its own span | |
| Key up → last segment transcribed | 60 / 120 ms | 48-71 ms | 2026-10-12, in the app, Quality, CUDA, 3 dictations of a 5.7 s recording |
| Key up → cleanup done | | 131-158 ms | same run |
| Paste (clipboard, keys, restore scheduled) | | 47-61 ms | same run |
| **Key up → text in the app, Quality, CUDA** | **400 / 800 ms** | **231-260 ms** | same run |
| Key up → text ready, `e2e-bench`, Quality, CUDA | 400 / 800 ms | 199 / 345 ms (p50 / p95) Windows; 197 / 393 ms Linux (WSL 2) | 2026-10-18, 40 clips of AMI and VoxPopuli |
| Key up → text ready, `e2e-bench`, Light, CUDA | 400 / 800 ms | 167 / 323 ms (p50 / p95) | 2026-10-11, 120 clips of AMI, VoxPopuli, Earnings-22 |
| Key up → text ready, Vulkan on the 3090 Ti | 400 / 800 ms | Quality 234 / 429 ms, Light 201 / 381 ms (p50 / p95) | 2026-10-13, 80 dictations; real AMD and Intel cards not measured |
| Key up → text ready, the processor | none set | Extra light 0.39 / 0.81 s, Light 0.50 / 1.15 s (p50 / p95) | 2026-10-18, Ryzen 9 9900X, 40 clips, int8 speech |
| Key up → text, macOS Metal | 400 / 800 ms | Quality 0.8-1.2 s, Light 0.15-0.5 s; the processor (Light) about 1.5 s | 2026-10-19/20, Mac mini M2 16 GB, by hand |
| Launch → ready to dictate (models on disk) | 3 s / 6 s | 3.54-3.57 s warm, 5.59-6.01 s right after a reboot | 2026-10-12, Windows, Quality |
| Launch → ready, macOS | 3 s / 6 s | 2.2-2.9 s warm | 2026-10-19/20, Mac mini M2 |
| GPU memory, Utter and llama-server at peak | | Quality 4,572 MiB, Light 3,085 MiB (CUDA); 5,690 and 4,143 MiB (Vulkan on the 3090 Ti) | 2026-10-11; 2026-10-13 |
| Idle RAM, Rust process | 150 MB | not measured | |

Where a budget is missed, the decision about what to change is written in section 14 with the
measurement:
- Quality on macOS misses the 400 ms budget on an M2 (0.8-1.2 s); Light meets it. Not yet
  decided.
- The processor has no budget: Quality is not offered there (its cleanup takes seconds), and
  Extra light exists for it (2026-10-18).
- The old rows "gate skips the LLM" (120 / 250 ms) and "Command Mode rewrite" (700 ms / 1.5 s)
  are gone: every dictation is cleaned, and Command Mode is not built.

---

## 6. Models

Chosen by measurement on Utter's own evals, not by reputation. The candidate surveys and the
first benches are in [docs/research/model-survey.md](docs/research/model-survey.md) and
[docs/evals.md](docs/evals.md). Languages: English now; Vietnamese, with code-switching, later
(Tai, 2026-10-09: English first).

### 6.1 Speech recognition

**parakeet-ultra** (Moondream's weights on NVIDIA's Parakeet TDT 0.6B v3 architecture,
CC-BY-4.0), exported to ONNX by `utter-speech export` and published at
[ductai199x/utter-parakeet-ultra-onnx](https://huggingface.co/ductai199x/utter-parakeet-ultra-onnx).
Run by Utter's own decoder in `crates/utter-asr` on onnxruntime 1.31. Never fine-tuned
(2026-10-10); the dictionary steers it by phrase boosting in the greedy decoder (+6 points of
term recall, 2026-10-10).

| Backend | Build (component) | Encoder file | Runs on |
|---|---|---|---|
| CUDA | 8-bit weights, MatMulNBits over the float16 graph (`parakeet-ultra-onnx-w8`) | 649 MB | encoder on the GPU (CUDA provider), decoder and joint on the CPU |
| Vulkan | float16 (`parakeet-ultra-onnx-fp16`) | 1.22 GB | onnxruntime's WebGPU plugin provider (Dawn over D3D12 or Vulkan) |
| Metal | float16 (`parakeet-ultra-onnx-fp16`) | 1.22 GB | the WebGPU provider built into onnxruntime's macOS build (Dawn over Metal) |
| CPU | int8 weights and activations (`parakeet-ultra-onnx-i8`) | 615 MB | the processor, threads = half the logical processors (three quarters with High) |
| Android | float16 `.tflite` with four input lengths | 1.33 GB | LiteRT on the phone's GPU (in progress) |

Why: on 11 public test sets (2026-10-09) it tied the most accurate open model, Qwen3-ASR-1.7B
(7.3% mean English WER each), and beat Parakeet v3 and Cohere Transcribe, at a fraction of
Qwen's latency. Utter's runtime gives the same transcripts as Transformers at 34 ms p50 a clip
warm on the 3090 Ti. Moondream's own engine was rejected: it sends usage telemetry. Each build
gives the same WER within noise: 8-bit costs about 5 ms a clip on CUDA and saves 0.66 GB;
WebGPU runs 8-bit 34 ms slower, so it takes float16; int8 is 6.5 times faster than the 8-bit
build on the processor (WER 6.29% against 6.09% over 500 clips).

Voice activity: **Silero VAD 6.2.3** on the CPU.

### 6.2 Cleanup

Fine-tuned on Utter's published dataset
[ductai199x/dictation-to-text](https://huggingface.co/datasets/ductai199x/dictation-to-text)
by one recipe (`utter-train`), exported to Q4_0 GGUF, published with their adapters at
[ductai199x/utter](https://huggingface.co/ductai199x/utter). Shipped files are named
`utter-<tier>-v<N>-<quantization>.gguf`. All three expect the same one-line system prompt,
`crates/utter-llm/prompts/cleanup-ft.md`.

| Model (setting) | Base | Training | File | Offered on | Ready to send, realistic set (Kokoro / unseen voices) |
|---|---|---|---|---|---|
| **Quality** (`quality`) | Gemma 4 E4B | LoRA r16, 1 epoch | `utter-quality-v2-q4_0.gguf`, 5.15 GB | a GPU (CUDA, Vulkan, Metal); the default | 88.1% / 80.1% |
| **Light** (`light`) | Gemma 4 E2B | LoRA r16, 1 epoch | `utter-light-v2-q4_0.gguf`, 3.35 GB | everywhere; recommended for smaller cards | 85.4% / 75.5% |
| **Extra light** (`mini`) | Gemma 3 1B | full fine-tune, 2 epochs, lr 2e-5 | `utter-mini-v1-q4_0.gguf`, 720 MB | the processor only | 79.7% / 67.0% |

Quality and Light: 2026-10-12, v2 on held-out speech. Extra light: 2026-10-14, the same 25,469
held-out dictations, retried outputs included (Light scored 85.7% / 74.6% in that run). Gemma 4
models are Apache-2.0; the 1B is under the Gemma Terms of Use.

Runtime: llama.cpp's `llama-server` b11535, the official build per backend (CUDA 13.4, Vulkan,
Metal, CPU), context 4,096 tokens, with `--swa-full` (prompt processing about 95 → 25 ms),
n-gram drafting from the prompt (3-token match, 6-token draft: another 25% off, same outputs)
and `-ub 128` (about 90 MB less GPU memory, same outputs and speed). On Vulkan it always gets
`--device` for the high-performance card.

Why fine-tuned small models: off-the-shelf models came first (Tai, 2026-10-09), and the
prompted Gemma 4 26B-A4B won the first bench (99% of the curated checks, 136 ms p50 on real
speech). Fine-tuned on Utter's data, the E4B beat it on every final-text eval at a third of its
size (2026-10-10). The headline metric is "ready to send": the share of dictations a careful
writer would send exactly as typed, judged dictation by dictation by the 26B against the
intended text, with a rubric calibrated on Tai's review; goal 90%. Details, data and the
history of runs: [docs/cleanup.md](docs/cleanup.md) and [docs/evals.md](docs/evals.md).

The prompted 26B (`gemma4-26b-a4b-qat`) and the prompted E4B (`gemma4-e4b-qat`) stay optional
components for the eval tools and are never offered.

### 6.3 Which model runs where

| Backend | Models offered | GPU memory needed (`utter_pipeline::gpu_need`) |
|---|---|---|
| CUDA | Quality, Light | 4.8 GB, 3.25 GB (measured peaks rounded up) |
| Vulkan | Quality, Light | 6.2 GB, 4.5 GB (3090 Ti peaks, until AMD and Intel are measured) |
| Metal | Quality, Light | Vulkan's figures, until measured |
| CPU | Light, Extra light | |

First run recommends the best model the card holds with about 1 GB to spare (`card_need`). A
model chosen for a backend it does not run on gives way: Quality on the processor runs Extra
light, Extra light with a GPU runs Light (`runtime_choice::model_on`).

---

## 7. Platform layer

Every OS call is in `crates/utter-platform`. The pipeline never calls an OS API directly. Status
and how each piece works in detail: [docs/platforms.md](docs/platforms.md).

| Job | Windows | Linux (GNOME, KDE; Wayland and X11) | macOS |
|---|---|---|---|
| Hotkey and shortcuts | Raw Input (`RIDEV_INPUTSINK`) to a message-only window on its own thread; not a `WH_KEYBOARD_LL` hook, which Windows stops calling while a WebView2 window has focus | evdev, every keyboard on one thread, through a udev rule giving the seat's user access (`uaccess`), installed by the `.deb` or by first run through pkexec | one listen-only CoreGraphics event tap; needs Accessibility |
| Insert | Clipboard (every global-memory format saved, kept out of clipboard history and cloud clipboard) plus `SendInput` paste keys; Unicode keystrokes for typed rules | Clipboard (arboard) plus a uinput virtual keyboard | Pasteboard saved whole plus Cmd+V as CoreGraphics events |
| Focus | Foreground window, its process, whether it runs elevated (no keys are sent to an administrator window from a normal Utter) | AT-SPI focus events (Qt apps once the session's accessibility switch is on, which Utter sets) | Frontmost application |
| Caret (text before the cursor) | UI Automation's text pattern | AT-SPI Text | Accessibility (`AXSelectedTextRange`) |
| Bar overlay | No focus, no taskbar button, click-through, bottom centre of the focused window's monitor | Placed under XWayland; a notification-type window so KWin never activates it | Transparent, non-activating |
| Start at login | HKCU Run entry (on by default for an installed copy) | XDG autostart entry | LaunchAgent, written only when it changes |
| GPU | NVIDIA driver (CUDA) or DXGI (Vulkan's card) | NVIDIA driver or Vulkan's device list | Metal's default device |

Known limits:
- Chromium and Electron apps on Linux answer AT-SPI only with accessibility turned on for them;
  Utter then still pastes but cannot check the window or space the text.
- Apps that do not report their text (most terminals) get no spacing and no paste
  confirmation; they are not second-guessed.
- macOS ties the keyboard permission to the app's signature: Utter is signed with its own
  certificate so the grant survives updates (section 14, 2026-10-19).
- The GPU clock hold works on NVIDIA only.

---

## 8. User interface

### 8.1 Surfaces

| Surface | What it is | Spec |
|---|---|---|
| **The Bar** | A small floating pill, hidden at idle, shown on the key press: listening (waveform and live text), processing, done, error, each with a one-line message. Never takes focus; lets every click through. | [docs/ux/bar.md](docs/ux/bar.md) |
| **Tray** (menu bar on macOS) | Open Utter, Paste last dictation, Copy last dictation, Retry last dictation, an Update item when one is offered, Quit | [docs/ux/home.md](docs/ux/home.md) |
| **Home** | One window with a rail: History, Dictionary, Settings | [docs/ux/home.md](docs/ux/home.md), [docs/ux/history.md](docs/ux/history.md) |
| **Paste rules** | Every rule in a searchable, sortable table; add any app | [docs/ux/insertion.md](docs/ux/insertion.md) |
| **Setup** | First run, and the Update, Repair and Problem screens | [docs/ux/first-run.md](docs/ux/first-run.md) |
| **Splash** | A frameless card while dictation loads after a manual start; never at sign-in | [docs/ux/splash.md](docs/ux/splash.md) |
| **Android bubble and pill** | In progress | [docs/ux/android.md](docs/ux/android.md) |

### 8.2 Feel

Each item is an acceptance check in the surface's spec.
- The Bar reacts to the key within one frame. The waveform follows the voice with no visible
  stutter, and live words never jump around.
- Motion has a purpose and is short (120-200 ms), with easing tokens from one design-token file
  (`app/src/lib/tokens.css`). No animation blocks input.
- Every state has an exit. Esc cancels. An error says what happened and what to do, in one line.
- Light and dark themes follow the OS. Text respects the OS scale. The Bar stays sharp at every
  DPI.
- Keyboard first in the Home window.
- No sounds yet (planned as optional start, stop and error cues).

### 8.3 Design process

Each surface gets a spec before it is built: layout, states, motion, copy, keyboard behavior
and acceptance checks. It is then built and reviewed against the spec in rounds, on the real
machine, until every check passes. Specs live in `docs/ux/`.

---

## 9. Features in detail

How each built feature behaves is in its spec: cleanup rules in
[docs/cleanup.md](docs/cleanup.md), insertion and paste rules in
[docs/ux/insertion.md](docs/ux/insertion.md), History in [docs/ux/history.md](docs/ux/history.md),
the Home window and settings in [docs/ux/home.md](docs/ux/home.md), first run in
[docs/ux/first-run.md](docs/ux/first-run.md). Scope (built, next, not built) is section 2.

Rules that hold across features:
- **A dictation is never lost.** It goes in, or it stays on the clipboard with a notice, and it
  is in History; the last one can always be pasted or copied again.
- **Settings apply without a restart**, on the next dictation: `settings.toml` is watched, so a
  hand edit applies like a change in the Home window. The dictation key restarts the listener
  at once (back to the old key if the new one cannot start). A model or runtime switch loads
  the new model beside the old one while dictation continues.
- **Updates never stop dictation.** A component update downloads while the earlier version
  runs, and takes over when it is whole.

---

## 10. Privacy

- Audio and text stay on the machine. History holds no audio and can be turned off; retention
  is forever, 30 days, 24 hours or off.
- No telemetry. Logs (`utter.log`, `llama-server.log`) hold timings, stages and errors, never
  what was said, and can be turned off (`keep_logs`), which deletes them. The dictation log with
  audio is kept only by development builds or with `debug_log` on.
- Network use: component and model downloads (GitHub, PyPI and Hugging Face, each file checked
  against its pinned SHA-256), and the update check (one request a day for the latest release's
  public `latest.json`, nothing about the person in it; off in Settings > About). An update is
  checked against the updater key built into the app before it runs.
- The text before the cursor is read only at key down and right after a paste, only from the
  focused field, and never leaves the process.

---

## 11. Storage

Per-user app data, never shared between an installed copy and development:

| | Windows | Linux | macOS |
|---|---|---|---|
| Installed | `%LOCALAPPDATA%\Utter` | `~/.local/share/utter` | `~/Library/Application Support/Utter` |
| Development and tools | `%LOCALAPPDATA%\Utter_dev` | `~/.local/share/utter_dev` | `~/Library/Application Support/Utter_dev` |

Inside: `settings.toml` (every setting, the dictionary and the paste rules; written with every
key commented on first start, edited by the Home window or by hand, comments kept),
`history.jsonl` (one JSON object per dictation, rewritten through a temporary file on edits),
`runtime/<id>-<version>/` (installed components), `downloads/`, `logs/` (rotating, capped),
`dictations/` (the dictation log, when kept). Uninstalling can keep or delete it.

---

## 12. Quality: what "done" means

1. **Tracing on every dictation.** Each stage is timed from key down and key up, in the logs
   and the dictation log. `utter-cli e2e-bench` replays eval clips through the real pipeline in
   real time (no insertion) and reports p50 and p95; `UTTER_TEST_AUDIO` feeds a recording
   through the whole app, insertion included, without playing sound.
2. **Evals.** Speech: WER on public test sets (AMI, Earnings-22, GigaSpeech, LibriSpeech,
   Voice Arena Monsoon, URGENT24, VoxPopuli; for Vietnamese, Bud500, FLEURS, GigaSpeech2,
   ViMD). Cleanup: dictation built from real written text, spoken in seven ways, voiced by TTS
   (Kokoro, and Chatterbox voices no training data uses) and recognized by the same Parakeet,
   with the answer key known by construction; ready to send judged by the 26B, plus exact checks
   by code (spans, dictionary terms, directed text typed not answered). Every change to a
   model, prompt or setting is run against them and logged in [docs/evals.md](docs/evals.md),
   regressions included. Method: [docs/evals.md](docs/evals.md); data: the published datasets
   (`dictation-to-text`, `dictation-speech`).
3. **Tests.** Logic is tested in Rust without the OS or GPU (the hotkey machine, the segmenter,
   the gate, the guards, settings, History, runtime choice). Tests that need models are marked
   and run on demand. CI (`.github/workflows/ci.yml`) runs the pages' build, format, clippy with
   warnings as errors and the tests on Windows, Ubuntu 24.04 and macOS.
4. **Platforms.** Each OS and package is checked end to end like a user: install, first run,
   dictation into real apps, update, quit leaving no process. Linux and Windows in throwaway
   VMs, macOS on a rented Mac mini M2, GPUs where they are
   ([docs/development.md](docs/development.md)).
5. **UX acceptance.** Each surface's spec in `docs/ux/` lists checks. It is done when every
   check passes on the real machine.
6. **Definition of done for a feature:** its latency budget met, its eval numbers at or above
   target, its UX checks passed, its docs updated in the same commit.

---

## 13. Repository layout

```
utter/
  DESIGN.md            this file
  CLAUDE.md            working conventions
  README.md            the public page: what Utter is, install, use
  crates/              the Rust core (section 3.2)
  app/                 Tauri shell (src-tauri/) and the Svelte pages (src/); brand/ holds the mark
  android/             the Android app, Kotlin, its own Gradle build
  tools/               Python: eval, training, datasets, benchmarks, media (uv; never shipped)
  packaging/           Linux packages (udev rule, scripts), macOS signing and cask, latest.json
  Casks/utter.rb       the Homebrew cask; this repository is the tap
  docs/                specs, evals, platform and development notes (index: docs/README.md)
  dev/                 the branch tooling: the privacy scanner and the publish to main
  media/               the README's pictures
  .github/workflows/   CI
```

Work happens on `dev`, which carries `docs/`, `DESIGN.md`, `CLAUDE.md` and `dev/`. `main` is
the public tree, built from `dev` by `dev/release.py publish` ([dev/README.md](dev/README.md)).

---

## 14. Decisions log

One line each, oldest first, with the measurement behind it. A decision later replaced stays,
marked "superseded by" the date of the entry that replaced it. "Where" points to this file's
sections or to the doc with the detail.

| Date | Decision | Where |
|---|---|---|
| 2026-10-09 | Push-to-talk is Right Ctrl; the other shortcuts follow Wispr Flow's Windows defaults. Amended 2026-10-13 (any of a list of keys) and 2026-10-19 (Right Option on a Mac). | 4.1 |
| 2026-10-09 | The Bar is hidden at idle and appears on the key press. | 8.1, [docs/ux/bar.md](docs/ux/bar.md) |
| 2026-10-09 | Text goes into the app once, at the end. Live words show in the Bar. | 4.2 |
| 2026-10-09 | Off-the-shelf models first, measured on our own bench; candidates from a live survey, never from memory. | 6, [docs/research/model-survey.md](docs/research/model-survey.md) |
| 2026-10-09 | Languages: English and Vietnamese, with code-switching. Parakeet v3 alone no longer fits (no Vietnamese). Amended the same day: English first. | 6 |
| 2026-10-09 | Eval on public datasets, no recordings from Tai. | 12, [docs/evals.md](docs/evals.md) |
| 2026-10-09 | English first: v1 ships the best English model; Vietnamese follows (a second, routed model or a later single one). Candidates: Qwen3-ASR-1.7B and Cohere Transcribe (tie, 7.2% and 7.4% mean WER). | 2.2, 6 |
| 2026-10-09 | English speech recognition: parakeet-ultra weights (CC-BY-4.0) in Utter's own ONNX runtime; ties the most accurate open model (7.3% mean WER) at a fraction of its latency. Never Moondream's engine, which sends telemetry. | 6.1, [docs/evals.md](docs/evals.md) |
| 2026-10-09 | The recognizer runs in the Rust core: encoder on the GPU (convolutions rewritten for fixed shapes), decoder on the CPU. Same accuracy as Transformers, 34 ms p50 warm. The pipeline warms the GPU on key down. | 4, 6.1, [docs/evals.md](docs/evals.md) |
| 2026-10-09 | Standalone: nothing but the GPU driver is taken from the machine. onnxruntime, CUDA, cuDNN and llama-server are pinned, checksummed components; llama-server is Utter's child process, ended when Utter dies. | 1, 3, [docs/development.md](docs/development.md) |
| 2026-10-09 | Cleanup model: Gemma 4 26B-A4B (QAT 4-bit) in llama-server with `--swa-full` and n-gram drafting: 99% of the cleanup checks, 101 ms p50 (136 ms on real speech). Latency over precision: slower, higher-precision variants are not pursued. Superseded by 2026-10-10 (fine-tuned E4B). | 6.2, [docs/evals.md](docs/evals.md) |
| 2026-10-09 | Footprint about 5 GB: the float16 speech encoder plus Gemma 4 E4B, about 5.6 GB of GPU memory; the 26B an optional tier. Context stays 4,096. Superseded by 2026-10-11 (Quality and Light, 8-bit speech, 4.8 GB). | 6, [docs/evals.md](docs/evals.md) |
| 2026-10-10 | Hold the GPU's clocks up from key down to key up with a napping one-thread kernel (CUDA driver API, no component), stopped at key up. Key up to text: AMI p50 55 → 31 ms, VoxPopuli 74 → 50 ms. About 157 W while the key is down, nothing at idle. | 4, [docs/evals.md](docs/evals.md) |
| 2026-10-10 | The segmenter cuts at a 128 ms pause once a segment is past 8 s (384 ms before), so the last segment stays short. | 4 |
| 2026-10-10 | The Bar shows the text as it will be typed, live: at every pause the dictation so far is cleaned and shown, words heard since are provisional, and the paste is exactly the last cleaned text. Every dictation is cleaned; the keyword gate is only logged (Tai's first live test). | 4.2, [docs/ux/bar.md](docs/ux/bar.md), [docs/cleanup.md](docs/cleanup.md) |
| 2026-10-10 | Cleanup is judged on an eval built from real written text, spoken in several kinds with gold known by construction, synthesized by TTS and recognized by Parakeet; never on hand-written examples. Fine-tuning the cleanup model is the expected path. | 6.2, 12, [docs/cleanup.md](docs/cleanup.md), [docs/research/2026-10-10-final-text.md](docs/research/2026-10-10-final-text.md) |
| 2026-10-10 | Parakeet is never fine-tuned; only the cleanup model is. The dictionary reaches recognition by phrase boosting in the greedy decoder: +6 points of term recall at no cost elsewhere. | 6.1, [docs/evals.md](docs/evals.md) |
| 2026-10-10 | Lists are formatted and grammar is fully repaired, both on by default; lists have a setting. | [docs/cleanup.md](docs/cleanup.md) |
| 2026-10-10 | The light-tier cleanup model is Gemma 4 E4B fine-tuned (ft4, LoRA, Q4_0, the short prompt `cleanup-ft.md`): main set 18.5% no edit needed against 7.2% (prompted E4B) and 11.3% (prompted 26B); lists 45.9% against 2.7% and 3.2%. Superseded by 2026-10-11 (ft5). | 6.2, [docs/evals.md](docs/evals.md) |
| 2026-10-11 | The cleanup model is judged by one number, ready to send (judged per dictation by Gemma 4 26B against the intended text, rubric calibrated on Tai's review), on a realistic set of everyday dictation; goal 90%. ft5: 88.8% against ft4's 83.8%, no regression elsewhere. | 6.2, 12, [docs/evals.md](docs/evals.md) |
| 2026-10-11 | Dictated text is typed, never answered or obeyed: ft5 answers none of 292 directed dictations; `cleanup::strayed` types the transcript if an output strays (no false alarm on 14,382 outputs). A self-correction inside such text is applied like any other. | 4, [docs/evals.md](docs/evals.md) |
| 2026-10-11 | Settings live in `settings.toml`, edited by the Home window or by hand, applied on the next dictation without a restart. Each cleanup request carries only the dictionary terms heard in it (`gate::relevant_terms`: exact under five letters, within a quarter of the letters otherwise). The dictionary stays in `settings.toml` (the planned SQLite store was never built). | 9, 11, [docs/ux/home.md](docs/ux/home.md) |
| 2026-10-11 | Two models, named for people: **Quality**, the fine-tuned E4B (the default), and **Light**, the fine-tuned E2B for 4 GB cards. First run recommends one for the GPU; Settings switches. The prompted 26B is never offered. Amended 2026-10-18 (Extra light). | 6, 8 |
| 2026-10-11 | Quality v1 is run ft6 (ft5 plus dictionary training): dictionary terms right 97.8% (ft5 96.2%), never echoes the dictionary line, within half a point with and without a dictionary. Superseded by 2026-10-12 (v2). | 6.2, [docs/evals.md](docs/evals.md) |
| 2026-10-11 | Everything on the GPU for both models (Tai), as lean as quality allows: the 8-bit speech encoder (same transcripts, about 5 ms slower), memory-pattern planning off, `-ub 128`. Quality 4.8 GB (was 5.6), Light 3.2 GB. A model is offered on a card with 1 GB more than it needs. Amended 2026-10-13 and 2026-10-18 (float16 on WebGPU, int8 on the processor). | 6, [docs/evals.md](docs/evals.md) |
| 2026-10-11 | Data lives where each platform keeps per-user app data: `Utter` for an installed copy, `Utter_dev` beside it for development and tools, same layout, never shared. | 11, [docs/development.md](docs/development.md) |
| 2026-10-11 | Windows installer: a per-user NSIS setup (3.9 MB, no admin prompt); first run downloads the models. Not code-signed, so SmartScreen warns. Start with Windows on by default, a switch in Settings. | [docs/platforms.md](docs/platforms.md) |
| 2026-10-11 | A dictation is never lost: copied when it cannot go in, paste and copy last, per-app paste rules, History. | 2.1, [docs/ux/insertion.md](docs/ux/insertion.md) |
| 2026-10-12 | onnxruntime sessions run 2 threads with no spin-waiting: 12 spinning threads cost audio (26 stream gaps) and time. Key up to text in the app 282-320 → 231-260 ms; launch to ready 3.55-3.75 s with the models loading side by side. | 5, [docs/evals.md](docs/evals.md) |
| 2026-10-12 | History is a JSON-lines file with no audio; the dictation log with audio is the developer's, kept only by development builds or `debug_log`. | 10, 11, [docs/ux/history.md](docs/ux/history.md) |
| 2026-10-12 | Code under AGPL-3.0-only; models and datasets carry their own licenses. | README |
| 2026-10-12 | The tools are five entry points around the published datasets (`utter-eval`, `utter-train`, `utter-data`, `utter-bench`, `utter-speech`). | [docs/development.md](docs/development.md) |
| 2026-10-12 | The cleanup models are v2, trained on the published `dictation-to-text` with one recipe for both tiers, so every shipped model is reproducible from public data. Quality v2 ties v1 on held-out speech (88.1% vs 88.5%), Light v2 trails by 1-2 points; standardization wins (Tai). Shipped in 0.1.2. | 6.2, [docs/evals.md](docs/evals.md) |
| 2026-10-12 | A component update keeps the old version until the new one is intact, and dictation runs on it meanwhile (0.1.3; the takeover without downtime 2026-10-13). | 9 |
| 2026-10-13 | The dictation key is chosen in Settings among keys that do nothing alone (Right Ctrl default, Left Ctrl, Right Shift, Scroll Lock, F13-F24); no Alt, Caps Lock, Insert or mouse buttons. Laptops without Right Ctrl could not dictate before. | 4.1 |
| 2026-10-13 | Pastes are confirmed by reading the text before the cursor again (up to 400 ms); a paste that does not show is copied, marked unconfirmed in History, and the Paste rules window offers a typed rule for that app. | 4, [docs/ux/insertion.md](docs/ux/insertion.md) |
| 2026-10-13 | The cleanup worker supervises llama-server (health check every 5 s, restarts after 1, 5, 30, 60 s) and answers at once while it is down, so the raw text goes in without the 3 s timeout. | 3.1 |
| 2026-10-13 | AMD and Intel cards run on a Vulkan backend, chosen when NVIDIA's driver does not answer: llama.cpp's official Vulkan build with `--device`, and the float16 encoder on onnxruntime's WebGPU plugin provider on the same card, each a pinned component, a runtime per backend. NVIDIA keeps CUDA: Vulkan costs 54 ms of key up to text at p50 on the 3090 Ti (234 vs 180 ms). Preview until measured on real AMD and Intel cards. Shipped in 0.1.6. | 3, 6.3, [docs/platforms.md](docs/platforms.md) |
| 2026-10-13 | Android: measure every runtime on the phone first; a Kotlin app around the Rust core through JNI; insertion through a bubble, a keyboard as the fallback (Tai). | 2.2, [docs/android.md](docs/android.md) |
| 2026-10-14 | A cleanup reply that runs away is retried once with a grammar forbidding a leading list marker and a mild repetition penalty, not typed raw. Light: 3 runaways in 25,469 held-out dictations, none left after the retry, scores unchanged. Shipped in 0.1.7. | 4 step 7, [docs/cleanup.md](docs/cleanup.md) |
| 2026-10-14 | Small cleanup models are fully fine-tuned, not LoRA: a rank-16 LoRA leaves a 1B its habit of opening lists and looping (806 cut-offs against 39). Gemma 3 270M is out (25 points behind Light). | [docs/android.md](docs/android.md) |
| 2026-10-16 | Android architecture: the encoder on the phone's GPU through LiteRT, called from Rust; decoder and joint on onnxruntime's CPU build; llama-server on the CPU with the 1B (1.49 s p50 a cleanup on the phone, Light 2.69 s). The desktop loop's platform-free half became `utter_pipeline::Session`, shared with `utter-mobile` (20 of 20 desktop outputs identical after the split). | 3.2, [docs/android.md](docs/android.md) |
| 2026-10-16 | Utter on Linux and macOS from the same codebase; macOS compile-checked in CI from the start; CI on GitHub Actions for Windows, Ubuntu and macOS. | 7, 12 |
| 2026-10-16 | macOS ships through Homebrew, unsigned (no Apple Developer ID). Superseded by 2026-10-19 (signed with Utter's own certificate). | [docs/platforms.md](docs/platforms.md) |
| 2026-10-18 | Linux: evdev for keys and uinput for the paste keys through a udev `uaccess` rule (the `.deb` installs it, first run offers it through pkexec), AT-SPI for focus and caret, the Bar under XWayland; GNOME and KDE, Wayland and X11; `.deb` and AppImage. Parity on the same GPU: Linux 197 ms p50 against Windows 199 ms, transcripts identical 40 of 40. | 7, [docs/platforms.md](docs/platforms.md) |
| 2026-10-18 | Utter runs without a graphics card: the processor is a backend, chosen by itself or in Settings ("Runs on" lists what the computer has). Quality is not offered there; **Extra light**, the phone's Gemma 3 1B, is, recommended: its cleanup is 21% faster than Light's at p50 on a desktop processor, and 1.5 s against 2.7 s on the phone. | 6.2, 6.3, [docs/platforms.md](docs/platforms.md) |
| 2026-10-18 | The processor, made fast: an int8 speech model of its own, speech threads a share of the logical processors (half, three quarters with High), no keep-alive passes. Extra light key up to text 1.84 → 0.37 s p50, 2.48 → 0.73 s p95 on a Ryzen 9 9900X. A Processor threads setting (half or three quarters of the threads) for the cleanup model. | 4, 6.1, [docs/platforms.md](docs/platforms.md) |
| 2026-10-19 | Long dictations cleaned in a 500-word window: tried and reverted the same day, as re-cleaning up to 500 words on every request is too slow. Cleanup that writes only its edits is the plan. | 2.2, [docs/cleanup.md](docs/cleanup.md) |
| 2026-10-19 | macOS is tested on a rented Mac mini M2 (macOS 26 and 27), not compile-checked only. Supersedes "no test machine" (2026-10-09 draft). | 12, [docs/development.md](docs/development.md) |
| 2026-10-19 | macOS keyboard access asks for Accessibility only (it covers reading keys and pasting; asking for Input Monitoring too added Utter to neither list); Utter restarts itself once it is granted. Right Option is the default key; Ctrl+Cmd+V and Ctrl+Cmd+C paste and copy the last dictation. | 4.1, 7, [docs/platforms.md](docs/platforms.md) |
| 2026-10-19 | The Mac app is signed with Utter's own certificate for every release (not Apple's Developer ID): with the ad-hoc signature every update lost the keyboard permission; with one certificate the grant survived updates, reinstalls and the in-app updater on macOS 26 and 27. | 7, [docs/development.md](docs/development.md) |
| 2026-10-19 | This repository is Utter's Homebrew tap: `Casks/utter.rb` at the root, written by `packaging/macos/cask.sh`, so `brew tap ductai199x/utter https://github.com/ductai199x/utter` then `brew install --cask utter` work with no second repository. | 13, [docs/development.md](docs/development.md) |
| 2026-10-20 | In-app updates: while "Check for updates" is on (the default), Utter asks GitHub for the latest release's `latest.json` once a day and only offers a newer version, in Settings and the tray; nothing downloads or installs until the person clicks Update; the download is checked against the updater key in the app. Windows runs the new installer, an AppImage replaces itself, a `.deb` installs through one pkexec prompt, macOS replaces Utter.app signed with the same certificate. Shipped in 0.1.10. | 2.1, 10, [docs/development.md](docs/development.md) |
| 2026-10-20 | Work happens on `dev`; `main` is the public tree, published from `dev` by `dev/release.py` after a privacy scan. Docs and this file live on `dev`. | 13, [dev/README.md](dev/README.md) |
