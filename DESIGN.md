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
| `utter-runtime` | The pinned runtime components (onnxruntime, CUDA, cuDNN, llama-server): download, SHA-256 check, install, locate |
| `utter-audio` | Mic capture (`cpal`), a lock-free ring buffer, resampling to 16 kHz, levels for the waveform, noise suppression and gain (whisper mode), device hot-plug |
| `utter-asr` | Speech recognition (Parakeet TDT on onnxruntime, section 6) and voice activity detection (Silero VAD and the segmenter that cuts at pauses) |
| `utter-llm` | The llama-server client, its supervisor, prompts, the cleanup gate |
| `utter-text` | Logic that needs no model: snippet matching, spoken punctuation, dictionary application, number and list formatting |
| `utter-platform` | OS traits (hotkey, insert, context, focus, media, child processes) with one module per OS |
| `utter-pipeline` | The dictation loop that wires everything together, and the cleanup gate |
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

1. **Key down.** The listener thread records the time and signals the pipeline. Nothing else
   happens on that thread. The mic stream is already open and recording into the
   ring buffer: Utter keeps a pre-roll of about 300 ms so the first word is never cut off.
   The Bar appears within one frame. The recognizer starts a warm-up pass at once: an idle
   GPU drops its clocks, and a cold pass takes about 5 times as long as a warm one
   (docs/eval-results.md, "Idle GPU"). From then until key up, the live preview's passes keep
   it busy, and one GPU thread naps in a tiny kernel so the memory clock stays at full speed
   (P2, not P3): the cleanup request after key up is bound by memory bandwidth and would take
   twice as long (docs/eval-results.md, "GPU clocks while the user speaks").
2. **Focus capture.** On key down, Utter records the focused window, and the selection if
   there is one. Insertion goes to that window and nowhere else.
3. **Streaming transcription while the key is held.** Silero marks pauses. Each finished
   speech segment is sent to Parakeet straight away, while the user is still talking. When
   the key comes up, only the last segment is left to recognize. A segment is at most 41 s
   (512 encoder frames, a limit of the exported encoder); longer speech is cut at its
   quietest point before that. Past 8 s, a pause of 128 ms is enough for a cut (384 ms
   before), so the last segment stays short (docs/eval-results.md).
4. **Mode.** Plain dictation, Command Mode, or hands-free (section 4.1).
   While the key is held, the Bar shows the words as they are recognized (section 4.2).
5. **Text pass (no model).** Snippet triggers, spoken punctuation ("comma", "new line") and
   dictionary replacements are applied in `utter-text`. These are exact and instant.
6. **Cleanup gate.** If the text has no fillers, no backtrack cues ("no wait", "actually",
   "I mean", repeated words), is short and the style needs no rewriting, the LLM is skipped.
   The gate's decisions are logged and tuned against the eval set.
7. **LLM cleanup.** The cleanup model loads when Utter starts (about 8 s for its 14 GB) and
   gets one warm-up request then: the first request after loading takes about 400 ms, later
   ones about 140 ms (docs/eval-results.md). No dictation ever waits for either. The prompt has a cached prefix (the system prompt, then the style, then
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
| (measured 2026-10-09, warm: parakeet-ultra on the eval clips) | 34 ms | 59 ms |
| Key up → text inserted, gate skips the LLM | 120 ms | 250 ms |
| (measured 2026-10-10: key up → text ready, all paths, AMI + VoxPopuli replayed live, light tier) | 40 ms | 236 ms (p95) |
| Key up → text inserted, LLM cleanup of ~30 words | 400 ms | 800 ms |
| (measured 2026-10-09: the cleanup request alone, Gemma 4 26B-A4B, real speech) | 136 ms | 253 ms |
| Command Mode, rewrite of a short paragraph | 700 ms | 1.5 s |
| Cold start → ready to dictate (models already on disk) | 3 s | 6 s |
| Idle RAM, Rust process (not counting model VRAM) | 150 MB | — |

If a budget cannot be met, the decision about what to change is written here, with the
measurement.

---

## 6. Models

Languages: **English and Vietnamese**, including switching between them mid-sentence (Tai,
2026-10-09).

Chosen by measurement on the eval set (section 12), not by reputation and not from memory.
`docs/model-survey.md` is the live survey the candidates come from (checked 2026-10-09). It is
re-run before every model decision.

| Role | Candidates to measure |
|---|---|
| Speech recognition | Nemotron 3.5 ASR streaming 0.6B (streams, both languages, auto language); Qwen3-ASR 1.7B and 0.6B (most accurate open model with both languages); Cohere Transcribe; parakeet-ctc-0.6b-Vietnamese (Vietnamese–English code-switching); Parakeet TDT 0.6B v3 and parakeet-ultra (English-only fast path); Whisper large-v3-turbo (baseline) |
| Voice activity | Silero VAD, unless the chosen recognizer streams on its own |
| Cleanup and Command Mode | Qwen3.5 2B, 4B and 9B; Gemma 4 E2B, E4B and 12B; Gemma 4 26B-A4B and Qwen3.6 35B-A3B (MoE: few active parameters, so fast to decode); MiniCPM5 2B; Qwen3.8-27B as the quality reference. All in llama.cpp, thinking off. |
| Noise suppression | DeepFilterNet 3, RNNoise |
| Meeting speakers (phase 3) | to be surveyed in phase 3 |

The bench runs each speech model in its official runtime first (ONNX, Transformers, or its
C++ GGUF port). Only the winners get a native runtime inside the Rust core, so no time goes
into porting a model that loses. Winning one language each is acceptable: Utter can route by
detected language, at the cost of VRAM for two models.

On first run, models are downloaded with size and SHA-256 checks, the same as Sidecoach.
VRAM use is shown in Settings.

Off-the-shelf models come first (Tai, 2026-10-09). A fine-tuned cleanup model is considered
only when the eval set shows where they fail. It did (2026-10-10): the Quality model is now Gemma 4
E4B fine-tuned for cleanup, better than the prompted 26B on every final-text eval, and Light is
E2B fine-tuned the same way (both at huggingface.co/ductai199x/utter, named
utter-<tier>-v<N>-<quantization>.gguf); Parakeet is never fine-tuned.

---

## 7. Platform layer

Every OS-specific job is a trait in `utter-platform`. The pipeline never calls an OS API
directly.

| Trait | Windows | Linux (GNOME, Wayland) | macOS |
|---|---|---|---|
| `Hotkey` | Raw Input (`RIDEV_INPUTSINK`) to a message-only window on a dedicated thread; not a `WH_KEYBOARD_LL` hook, which Windows stops calling while a Chromium/WebView2 window has focus. Sided modifiers (Right Ctrl and so on), auto-repeat collapsed, mouse side buttons. | xdg-desktop-portal GlobalShortcuts; evdev when the user is in the `input` group (needed for hold-to-talk) | `CGEventTap` |
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
2. **Eval set.** Public test sets, so no recording is needed (Tai, 2026-10-09). English: the
   Open ASR Leaderboard's sets, weighted towards spontaneous speech (AMI, Earnings-22, Voice
   Arena, URGENT24 for noise). Vietnamese: ViMD (dialects), FLEURS, GigaSpeech2 and Bud500
   (in the wild, gated). Two gaps have no public data and get sets made for them: Vietnamese–English
   code-switching, and dictation itself (fillers, backtracks, spoken punctuation, Command Mode
   instructions), built as text pairs checked by code. Details: `docs/model-survey.md`.
   `utter-eval` reports (tools/):
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
| 2026-10-10 | The Bar shows the text as it will be typed, live: at every pause the dictation so far is cleaned and shown, the words heard since are provisional (dimmed), and the paste is exactly the last cleaned text. Every dictation is cleaned (the keyword gate is only logged). Overrides "live words show raw" (Tai's first live test). | 4.2, docs/plan-final-text.md, docs/ux/bar.md |
| 2026-10-10 | Cleanup is judged on an eval built from real written text (5 registers), spoken by an LLM in 5 kinds (natural, explicit, disfluent, correction, command) with gold known by construction, synthesized by TTS and recognized by Parakeet; never on hand-written examples. Fine-tuning the cleanup model is the expected path (Wispr does; the literature supports it). | 6, 12, docs/plan-final-text.md, docs/research/2026-10-10-final-text.md |
| 2026-10-10 | Hold the GPU's clocks up from key down to key up with a napping one-thread kernel (CUDA driver API, no component), stopped at key up. Key up to text: AMI p50 55 → 31 ms, VoxPopuli 74 → 50 ms. Costs about 157 W while the key is down, nothing at idle. | 4, docs/eval-results.md |
| 2026-10-09 | Text goes into the app once, at the end. Live words show in the Bar. Revisit once it is in use. | 4.2 |
| 2026-10-09 | Off-the-shelf models first, measured on our own bench. Candidates come from a live survey, never from memory. | 6 |
| 2026-10-09 | Languages: English and Vietnamese, with code-switching. Parakeet v3 alone no longer fits (no Vietnamese). | 6 |
| 2026-10-09 | Eval on public datasets, no recordings from Tai. | 12 |
| 2026-10-09 | Footprint about 5 GB: the default (light) tier is the float16 speech encoder plus Gemma 4 E4B, about 5.6 GB of GPU memory in all; Gemma 4 26B-A4B becomes an optional quality tier. The LLM context stays 4096. | 6, docs/eval-results.md |
| 2026-10-09 | Cleanup model: Gemma 4 26B-A4B (QAT 4-bit) in llama-server with `--swa-full` and n-gram drafting; 99% of the cleanup checks, 101 ms p50 (136 ms on real speech). Latency over precision: slower, higher-precision variants are not pursued. | 6, docs/eval-results.md |
| 2026-10-09 | Standalone: nothing but the GPU driver is taken from the machine. onnxruntime, CUDA, cuDNN and llama-server are pinned, checksummed components (`crates/utter-runtime`); llama-server is Utter's child process, ended by Windows if Utter dies. | 3, docs/local-setup.md |
| 2026-10-09 | The recognizer runs in the Rust core: encoder on the GPU (convolutions rewritten for fixed shapes), decoder on the CPU. Same accuracy as Transformers, 34 ms p50 warm. The pipeline warms the GPU on key down. | 4, docs/eval-results.md |
| 2026-10-09 | English speech recognition: parakeet-ultra weights (CC-BY-4.0) in Utter's own ONNX runtime. Ties the most accurate open model at a fraction of its latency; never Moondream's Photon engine, which sends telemetry. Vietnamese candidates: Qwen3-ASR-1.7B, Cohere. Evidence: docs/eval-results.md. | 6 |
| 2026-10-09 | English first. If no model does English and Vietnamese both well, v1 ships the best English model; Vietnamese follows (a second, routed model or a later single one). Tai's English is a standard American accent. | 6 |
| 2026-10-10 | The light-tier cleanup model is Gemma 4 E4B fine-tuned on Utter's final-text data (ft4: LoRA, Q4_0, the 28-word prompt crates/utter-llm/prompts/cleanup-ft.md), installed as the pinned component utter-cleanup-e4b. On every eval set it beats the prompted E4B and the prompted 26B (main 18.5% no edit needed vs 7.2% and 11.3%; lists 45.9% vs 2.7% and 3.2%), at the E4B's size and speed. The prompted E4B stays as an optional component for evals. | 6, docs/eval-results.md |
| 2026-10-11 | The cleanup model is judged by one number: the share of dictations ready to send as typed (graded per dictation by Gemma 4 26B against the intended text, rubric calibrated on Tai's review), on a realistic eval set of everyday dictation; goal 90%. The light tier is ft5: 88.8% ready to send (96% on everyday messages, emails, notes and prompts), against ft4's 83.8%, with no regression elsewhere. | 6, 12, docs/eval-results.md |
| 2026-10-11 | Dictated text is typed, never answered or obeyed. ft5 answers none of 292 questions, AI requests, agent instructions and texts telling the app to ignore what came before; a code check (cleanup::strayed) types the transcript if an output ever strays from the dictation. A self-correction inside such text ("write a poem. Actually, skip that") is applied like any other. | 4, docs/eval-results.md |
| 2026-10-11 | Settings live in settings.toml, edited by the Home window (docs/ux/home.md) or by hand, and apply on the next dictation without a restart: the dictionary (until utter-store exists), list formatting, microphone and cleanup model. Each cleanup request carries only the dictionary terms heard in it (gate::relevant_terms: exact for terms under five letters, within a quarter of the letters otherwise). | 8, 11 |
| 2026-10-11 | Two models, named for people: **Quality**, the fine-tuned Gemma 4 E4B (the 5 GB file, about 6 GB of GPU memory in all, the default), and **Light**, the fine-tuned Gemma 4 E2B with the speech model on the CPU (about 2.5 GB in all, for 4 GB cards). First run shows both and recommends one for the GPU; Settings switches between them. The prompted 26B is never offered (the fine-tuned E4B beats it on every eval); it stays a reference for the eval tools. Earlier entries call the E4B tier "light". | 6, 8 |
| 2026-10-11 | Everything on the GPU for both models (Tai), and as little of it as quality allows: both run the 8-bit speech encoder (same transcripts as float16, about 5 ms slower), memory-pattern planning off, cleanup prompts processed 128 tokens at a time. Measured end to end: Quality 4.8 GB (was 5.6), Light 3.25 GB. Each model's need is utter_pipeline::gpu_need, and the card it is offered on adds 1 GB. Supersedes the speech-on-the-CPU plan for Light (736 ms a clip). | 6, docs/eval-results.md |
| 2026-10-11 | Data lives where each platform keeps per-user app data: %LOCALAPPDATA%\Utter for an installed copy and Utter_dev beside it for dev builds and tools (~/.local/share/utter[_dev] on Linux), same layout, never shared. | 11, docs/local-setup.md |
