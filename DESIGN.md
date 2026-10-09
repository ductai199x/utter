# Utter: design

Utter is voice dictation that runs entirely on your own machine. Hold a key, speak, let go:
clean, well-punctuated text appears in whatever app has the cursor. It is a local answer to
Wispr Flow, built to feel instant.

This file is the source of truth for scope and decisions. A change of architecture changes
this file first. Status: **draft for review** (2026-10-09). Nothing is built yet.

---

## 1. Principles

1. **Local only.** No audio, text, context or telemetry leaves the machine. The network is
   used only to download models and app updates, and both are shown to the user.
2. **Latency is the product.** Every stage has a budget (section 5). Every stage is measured
   on every dictation. Nothing counts as "fast" until it is measured.
3. **Never lose words.** Audio capture starts before anything else. A failure at any stage
   still inserts the best text Utter has, or keeps it in History with a retry button.
4. **Never get in the way.** Utter never takes focus from the app being typed into. It never
   leaves the clipboard changed. It never types into the wrong window.
5. **No slop.** A feature is done when it meets its latency budget, its eval numbers and its
   UX acceptance criteria (section 12). "It works" is the starting point, not the finish.

---

## 2. Scope and phases

One codebase for Windows, Linux and macOS. Android is a separate Kotlin app, built last.

| Phase | Features | Platforms |
|---|---|---|
| 1 | Transcription, cleanup, backtrack, styles, Command Mode | Windows |
| 2 | Context awareness, text insertion polish, whisper mode | Windows |
| 3 | Notetaker, IDE file tagging, History and Stats | Windows |
| 4 | The same feature set | Linux (GNOME first) |
| 5 | The same feature set, compile-checked in CI only | macOS (no test machine) |
| 6 | Dictation keyboard and bubble | Android (Kotlin, separate) |

The dictionary and snippets are part of cleanup, so they arrive in phase 1. Basic insertion
(paste, then restore the clipboard) also arrives in phase 1, because nothing works without it.
Phase 2 makes insertion robust.

**Out of scope:** teams, admin and enterprise features, cloud sync, accounts.

---

## 3. Architecture

### 3.1 Processes

```
┌──────────────────────────── utter (one Tauri process) ────────────────────────────┐
│                                                                                    │
│  WebView windows (Svelte 5)          Rust core (crates/, in-process)               │
│  ┌───────────┐ ┌──────────────┐      ┌────────┐  ┌───────┐  ┌──────────┐            │
│  │ Bar       │ │ Settings,    │◄────►│pipeline│◄─┤ audio │◄─┤ hotkey   │ OS hooks   │
│  │ (overlay) │ │ History, ... │ IPC  │ (state │  └───────┘  └──────────┘            │
│  └───────────┘ └──────────────┘      │machine)│──► asr (ort + CUDA, in-process)     │
│                                      │        │──► context (UIA / AT-SPI / AX)      │
│                                      │        │──► insert (paste / keystrokes)      │
│                                      │        │──► store (SQLite)                   │
│                                      └───┬────┘                                     │
└──────────────────────────────────────────┼─────────────────────────────────────────┘
                                           │ HTTP on 127.0.0.1, OpenAI-compatible API
                                  ┌────────▼────────┐
                                  │  llama-server   │  child process, kept warm,
                                  │  (llama.cpp)    │  restarted by the core if it dies
                                  └─────────────────┘
```

- **Rust core in the Tauri process.** Audio, the hotkey, speech recognition, insertion and
  storage share one process. That means no IPC hop and no sidecar start-up on the hot path.
- **llama-server is a separate process.** The LLM is the largest and least stable component.
  As a child process, a crash cannot take the app down, a model swap is a restart, and
  prebuilt CUDA builds are available. The core supervises it with a health check, a restart
  and a warm-up request.
- **The UI only renders.** It shows state and sends user intent. All logic, timing and
  decisions stay in Rust, so they can be tested without a window.

### 3.2 Crates

| Crate | Job |
|---|---|
| `utter-core` | Shared types, config, the event bus, tracing setup |
| `utter-audio` | Mic capture (`cpal`), a lock-free ring buffer, resampling to 16 kHz, levels for the waveform, noise suppression and gain (whisper mode), device hot-plug |
| `utter-vad` | Silero VAD (onnx) |
| `utter-asr` | The speech-recognition trait. Parakeet (ort + CUDA) is the main implementation. Whisper (`whisper-rs`) is the multilingual fallback. |
| `utter-llm` | The llama-server client, its supervisor, prompts, the cleanup gate |
| `utter-text` | Logic that needs no model: snippet matching, spoken punctuation, dictionary application, number and list formatting |
| `utter-platform` | OS traits (hotkey, insert, context, focus, media) with one module per OS |
| `utter-pipeline` | The dictation state machine that wires everything together |
| `utter-store` | SQLite: history, dictionary, snippets, styles, stats |
| `src-tauri` | The shell: windows, tray, IPC commands, the updater |

Python lives in `tools/` only: the eval harness, dataset generation and benchmarks. It is
never shipped.

---

## 4. A dictation, step by step

```
idle ──key down──► listening ──key up──► finishing ──► cleaning ──► inserting ──► idle
                      │                                    │
                      │ Esc / key held < 150 ms             └─ gate says clean: skip
                      ▼
                   cancelled
```

1. **Key down.** The hook thread records the time and signals the pipeline. Nothing else
   happens inside the hook callback. The mic stream is already open and recording into the
   ring buffer: Utter keeps a pre-roll of about 300 ms so the first word is never cut off.
   The Bar appears within one frame.
2. **Focus capture.** On key down, Utter records the focused window, and the selection if
   there is one. Insertion goes to that window and nowhere else.
3. **Streaming transcription while the key is held.** Silero marks pauses. Each finished
   speech segment is sent to Parakeet straight away, while the user is still talking. When
   the key comes up, only the last segment is left to recognize.
4. **Mode.** Plain dictation, Command Mode, or hands-free (section 4.1).
   While the key is held, the Bar shows the words as they are recognized (section 4.2).
5. **Text pass (no model).** Snippet triggers, spoken punctuation ("comma", "new line") and
   dictionary replacements are applied in `utter-text`. These are exact and instant.
6. **Cleanup gate.** If the text has no fillers, no backtrack cues ("no wait", "actually",
   "I mean", repeated words), is short and the style needs no rewriting, the LLM is skipped.
   The gate's decisions are logged and tuned against the eval set.
7. **LLM cleanup.** The prompt has a cached prefix (the system prompt, then the style, then
   the dictionary terms) followed by the transcript and a little context. It runs with
   prompt-lookup decoding, because the output mostly copies the input. A guard rejects an
   output that drifts too far from the input (for example, answering a question instead of
   transcribing it) and inserts the text-pass result instead.
8. **Insertion.** Utter saves the clipboard, sets the text, sends a paste, then restores the
   clipboard. If the target window lost focus or no longer exists, nothing is typed: the
   text goes to History and the Bar says so.
9. **Record.** History saves the audio, raw transcript, final text, app, mode and per-stage
   timings.

### 4.1 Shortcuts

The defaults follow Wispr Flow's Windows layout, except that push-to-talk is Right Ctrl
(Tai, 2026-10-09). Each action takes up to four shortcuts, and mouse buttons count.

| Action | Default (Windows) | Wispr Flow's Windows default |
|---|---|---|
| Push-to-talk (hold) | Right Ctrl | Ctrl + Win |
| Hands-free (start and stop) | Double-tap Right Ctrl | Ctrl + Win + Space |
| Command Mode (hold) | Ctrl + Win + Alt | Ctrl + Win + Alt |
| Paste last dictation | Shift + Alt + Z | Shift + Alt + Z |
| Cancel | Esc | Esc |

Right Ctrl is also half of ordinary shortcuts (Right Ctrl + C). So a key press that joins
Right Ctrl while it is held turns that press into a normal shortcut: the dictation is
cancelled without a sound, and the key goes through to the app untouched. A hold shorter
than 150 ms with no speech is a tap and records nothing.

### 4.2 Live text: in the Bar, not in the field

Text is inserted into the app **once**, at the end. This is also what Wispr Flow does: its
text arrives in one paste when the key is released. Typing into another app while the user
speaks, then revising it, fights the app (autocomplete, chat apps that send on Enter, the
undo history) and makes Ctrl+Z undo one fragment instead of the dictation.

What makes it feel live is the Bar: during listening it shows the words as each segment is
recognized, and the words already shown never jump around. Typing into the field during long
hands-free sessions stays a possible later experiment, behind a setting, if the in-Bar preview
is not enough.

---

## 5. Latency budget

Measured on the development machine (RTX 3090 Ti). Each row has a trace span. p99 counts.

| Step | p50 | p99 |
|---|---|---|
| Key down → Bar visible and audio confirmed | 16 ms | 33 ms |
| Key up → last segment transcribed | 60 ms | 120 ms |
| Key up → text inserted, gate skips the LLM | 120 ms | 250 ms |
| Key up → text inserted, LLM cleanup of ~30 words | 400 ms | 800 ms |
| Command Mode, rewrite of a short paragraph | 700 ms | 1.5 s |
| Cold start → ready to dictate (models already on disk) | 3 s | 6 s |
| Idle RAM, Rust process (not counting model VRAM) | 150 MB | — |

If a budget cannot be met, the decision about what to change is written here, with the
measurement.

---

## 6. Models

Chosen by measurement on the eval set (section 12), not by reputation. These are the starting
candidates.

| Role | Main candidate | Alternatives to measure |
|---|---|---|
| Speech recognition | Parakeet TDT 0.6B v3 (25 European languages), ort + CUDA | Whisper large-v3-turbo for other languages, Canary |
| Voice activity | Silero VAD | — |
| Cleanup and Command Mode | Qwen3.5-4B, Q6, llama.cpp, thinking off | Qwen3.5-9B; Qwen3.5-2B for the fastest path; Qwen3.8-27B (Q4, about 16 GB) as the quality reference and a candidate for Command Mode |
| Noise suppression | DeepFilterNet 3 | RNNoise (lighter) |
| Meeting speakers (phase 3) | sherpa-onnx diarization | pyannote |

On first run, models are downloaded with size and SHA-256 checks, the same as Sidecoach.
VRAM use is shown in Settings.

Off-the-shelf models come first (Tai, 2026-10-09). A fine-tuned cleanup model is considered
only when the eval set shows where they fail. Qwen3.8 is the newest family, but its smallest
model is 27B (checked on Hugging Face, 2026-10-09). A dense 27B decodes several times slower
than a 4B, so it probably cannot meet the 400 ms cleanup budget; the bench decides that.

---

## 7. Platform layer

Every OS-specific job is a trait in `utter-platform`. The pipeline never calls an OS API
directly.

| Trait | Windows | Linux (GNOME, Wayland) | macOS |
|---|---|---|---|
| `Hotkey` | `SetWindowsHookEx` low-level keyboard and mouse hooks on a dedicated thread. Sided modifiers (Right Ctrl and so on), auto-repeat collapsed, mouse side buttons. | xdg-desktop-portal GlobalShortcuts; evdev when the user is in the `input` group (needed for hold-to-talk) | `CGEventTap` |
| `Insert` | Clipboard plus `SendInput` Ctrl+V. Fallback: `SendInput` Unicode keystrokes. | Portal RemoteDesktop or uinput; `wl-clipboard` | Pasteboard plus `CGEvent` Cmd+V |
| `Context` | UI Automation: app, window title, text near the caret, selection | AT-SPI | Accessibility API (`AXUIElement`) |
| `Focus` | Foreground window handle and process | Limited on Wayland: portal plus AT-SPI | `NSWorkspace` plus AX |
| `Media` | Global media session (pause and resume music) | MPRIS | `MediaRemote` (private) or skip |
| `SystemAudio` (phase 3) | WASAPI loopback | PipeWire monitor | ScreenCaptureKit |

Known hard parts, noted now so the traits fit them later:
- **Wayland** blocks global key hooks and synthetic input for normal apps. Hold-to-talk
  needs key-up events, which the GlobalShortcuts portal provides on recent GNOME. Older
  setups need evdev. Setup must explain this, not fail silently.
- **Secure input** (password fields, UAC prompts on Windows, Secure Event Input on macOS)
  blocks hooks and insertion. Utter detects it and says so in the Bar.
- **Elevated windows** on Windows ignore input from a non-elevated process. Detect and say so.
- **macOS** has no test machine. CI builds and runs the unit tests on a macOS runner. Manual
  testing waits for hardware.

---

## 8. User interface

### 8.1 Surfaces

| Surface | What it is |
|---|---|
| **The Bar** | A small floating pill. Hidden at idle (Tai, 2026-10-09), it appears on the key press. States: listening (live waveform and the words so far), processing, done (a short confirmation, then it hides), error. It never takes focus and lets clicks through except on its own controls. It can be dragged and docked. |
| **Tray** | Open, pause or resume, quit |
| **Home window** | History, dictionary, snippets, styles, stats, settings: one window with a sidebar |
| **First run** | Microphone choice and a level check, hotkey choice with a live test, model download with progress, then a guided first dictation |
| **Notetaker** (phase 3) | Meeting list, live transcript, summary, editor |

### 8.2 Feel

Each item becomes an acceptance check in the surface's spec.
- The Bar reacts to the key within one frame. The waveform follows the voice at the display's
  refresh rate with no visible stutter.
- Motion has a purpose and is short: 120–200 ms, with easing tokens from a single design-token
  file. No animation blocks input.
- Optional sounds for start, stop and error. They are short and quiet, and can be turned off.
- Every state has an exit. Esc cancels. An error says what happened and what to do, in one line.
- Light and dark themes follow the OS. Text sizes respect the OS scale. The Bar stays sharp at
  every DPI.
- Keyboard first: every action in the Home window can be reached without the mouse. Screen
  readers can use it.

### 8.3 Design process

Each surface gets a spec before it is built: layout, states, motion, sounds, copy and keyboard
behavior. It is then built and reviewed against the spec in rounds, live on the real machine,
until it passes. Specs live in `docs/ux/`.

---

## 9. Features

### Phase 1
- **Transcription.** Hold to talk, double-tap for hands-free, Esc to cancel. Language auto or
  pinned. Sessions up to 20 minutes, with segments transcribed as they finish.
- **Cleanup.** Removes fillers and fixes punctuation and casing. A cleanup level from "verbatim"
  to "polished".
- **Backtrack.** Applies corrections made mid-sentence ("at 2, no, 3" becomes "at 3"). Turns
  spoken numbers into lists.
- **Styles.** A tone per app or app category (Formal, Neutral, Casual, Very casual), plus your
  own instructions. The app comes from the focused process. Sites inside a browser come from
  the window title until context awareness arrives.
- **Command Mode.** With a selection: speak an instruction, and the selection is replaced
  (Ctrl+Z undoes it). With no selection: the answer is inserted. Saved commands ("Transforms")
  can be run by name.
- **Dictionary and snippets.** Manual entries, CSV import, and the dictionary terms passed to
  both recognition (as hints) and cleanup.

### Phase 2
- **Context awareness.** Text near the caret and the selection through accessibility APIs,
  used to spell names and match tone. Never screenshots. The context sent to the LLM can be
  viewed per dictation in History.
- **Insertion polish.** Per-app rules (some apps need typing, not pasting), terminals,
  remote-desktop windows, elevated windows, and checking that the paste landed.
- **Whisper mode.** Noise suppression, gain and a lower VAD threshold, switchable from the Bar.
- **Dictionary learning.** After insertion, Utter re-reads the field a few seconds later. If
  the user corrected a word, it offers to add that word.

### Phase 3
- **Notetaker.** Mic plus system audio, live transcript, speaker labels, a summary from the
  local LLM, and a local MCP server so Claude and other tools can query meetings.
- **IDE file tagging.** In VS Code and Cursor, a spoken file name is matched against the
  workspace's files and inserted as `@path`. Code words keep their casing (camelCase,
  snake_case).
- **History and Stats.** Search, audio playback, retry with another model or style, and copy.
  Words per day, words per minute and top apps. Retention settings, and a one-click delete.

---

## 10. Privacy

- Audio and text stay on the machine. History can be turned off. Retention can be set from
  "never keep" to "keep forever".
- No telemetry. Crash reports are written locally and shared only by the user, by hand.
- Network use: model downloads (Hugging Face, checked by SHA-256) and the app updater. Both
  can be turned off. Settings lists every host Utter contacts.
- Context awareness reads only the focused field, and only while dictating.

---

## 11. Storage

Under the app's data folder: `utter.sqlite` (history, dictionary, snippets, styles, stats),
`audio/` (History recordings, pruned by retention), `models/`, `logs/` and `traces/` (rolling).
Settings are in `settings.toml`, which the Home window edits and which a person can edit too.

---

## 12. Quality: what "done" means

1. **Tracing from the first commit.** Every dictation emits a trace (the `tracing` crate)
   with one span per stage. A debug overlay shows the latest breakdown live. `utter bench`
   replays recorded audio through the real pipeline and reports p50 and p99 per stage.
2. **Eval set.** About 200 of Tai's own recordings to start: plain dictation, fillers,
   backtracks, lists, jargon and names, code words, whispering, noise, other languages, and
   Command Mode instructions. Each has its expected text. `tools/eval` reports:
   - word error rate of recognition;
   - cleanup quality: exact checks where possible (fillers gone, backtrack applied, the
     dictionary obeyed), a judge model only for what code cannot decide;
   - the gate's skip rate and its error rate;
   - latency.
   Every change to a model, prompt or gate is run against it, and results are logged in
   `docs/eval-results.md`.
3. **Tests.** Logic is tested in Rust without OS or GPU (`utter-text`, the pipeline state
   machine with fake ASR and LLM). Tests that need the GPU or models are marked and run on
   demand. Platform code has a small manual checklist per OS.
4. **UX acceptance.** Each surface's spec in `docs/ux/` lists checks. It is done when every
   check passes on the real machine.
5. **Definition of done for a feature:** its latency budget met at p99, its eval numbers at or
   above target, its UX checks passed, its docs updated.

---

## 13. Repository layout

```
utter/
  DESIGN.md            this file
  CLAUDE.md            working conventions
  crates/              the Rust core (section 3.2)
  app/                 Tauri shell (src-tauri/) and the Svelte UI (src/)
  tools/               Python: eval, datasets, benchmarks (uv)
  docs/                ux/ specs, eval-results.md, platform notes
  android/             phase 6, Kotlin, its own Gradle build
```

---

## 14. Decisions log

| Date | Decision | Where |
|---|---|---|
| 2026-10-09 | Push-to-talk is Right Ctrl. The other shortcuts follow Wispr Flow's Windows defaults. | 4.1 |
| 2026-10-09 | The Bar is hidden at idle and appears on the key press. | 8.1 |
| 2026-10-09 | Text goes into the app once, at the end. Live words show in the Bar. (Pending Tai's confirmation.) | 4.2 |
| 2026-10-09 | Off-the-shelf models first: Qwen3.5 small models, measured against Qwen3.8-27B. | 6 |
