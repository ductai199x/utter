<p align="center">
  <img src="app/brand/icon.svg" width="96" alt="Utter">
</p>

<h1 align="center">Utter</h1>

<p align="center">
  <b>Hold a key, speak, and clean text appears in the app you are typing in.</b><br>
  Voice dictation that runs entirely on your own computer.
</p>

<p align="center">
  <a href="https://github.com/ductai199x/utter/releases/latest">Download for Windows</a> ·
  <a href="DESIGN.md">Design</a> ·
  <a href="docs/eval-results.md">Eval results</a> ·
  <a href="https://huggingface.co/ductai199x/utter">Models</a> ·
  <a href="https://huggingface.co/datasets/ductai199x/dictation-to-text">Datasets</a>
</p>

---

Hold **Right Ctrl**, talk the way you talk, let go. "Hey Sarah, I can take three days off starting
the fifteenth, no wait, fourteenth" becomes *Hey Sarah, I can take three days off starting the
14th.* in your email, chat, editor or terminal, about a quarter of a second after you release
the key. Nothing you say leaves the machine.

## What it does

- **Dictation that reads like writing.** Fillers go, punctuation and capitals come in,
  self-corrections ("three, no, four") are applied, spoken lists become bullets, spoken
  punctuation ("comma", "new line") is typed as marks.
- **Typed, never answered.** Dictate a question, an instruction to an AI assistant, or text
  that tells the app to "ignore everything before": it is typed as you said it, never obeyed.
  A check in the code types the transcript itself if the model's output ever strays from it.
- **Your words, spelled your way.** A dictionary of names and terms steers both recognition
  and cleanup.
- **Never loses a dictation.** If the text cannot go in (the focus moved, an admin window), it
  stays on the clipboard and the Bar says so. Alt+Shift+Z pastes the last dictation again. Your
  clipboard is always put back.
- **Fits the app.** Paste rules per app (terminals, remote desktops), automatic spacing when a
  dictation continues text, hands-free mode with a double-tap, Esc to cancel.
- **History.** Every dictation, searchable, with what Utter heard and what it changed. Kept on
  your computer only, for as long as you choose, or not at all.

## Measured, not claimed

On the development machine (RTX 3090 Ti), in the app itself. Every figure is from
[docs/eval-results.md](docs/eval-results.md), where the method and the raw numbers live.

| | |
|---|---|
| Key up to text in the app | **231-260 ms** (speech 48-71 ms, cleanup 131-158 ms, paste 47-61 ms) |
| Launch to ready | **3.5-3.8 s** warm, 5.6-6.0 s right after a reboot |
| Ready to send as typed, realistic dictation (Quality) | **88.1%**, 80.1% with voices the model never heard |
| Dictionary terms spelled right | **97.8%** |
| GPU memory | Quality **4.8 GB**, Light **3.2 GB** |

"Ready to send" is the share of dictations a careful writer would send exactly as typed, judged
dictation by dictation against the intended text with a rubric checked against human review.

## Install

Download `Utter_<version>_x64-setup.exe` from the
[latest release](https://github.com/ductai199x/utter/releases/latest) and run it. It installs
for your user only. On first start Utter checks your computer, recommends a model for your
graphics card, tests your microphone, downloads what it runs on, and ends with a test
dictation.

**Needs:** Windows 10 or 11 (64-bit), an NVIDIA graphics card with 4 GB of memory (6 GB for
Quality) and driver 580 or newer, and about 10 GB of disk. macOS and Linux are planned.

| Model | What it is | GPU memory |
|---|---|---|
| **Quality** | Gemma 4 E4B, fine-tuned for dictation cleanup | 4.8 GB |
| **Light** | Gemma 4 E2B, fine-tuned the same way | 3.2 GB |

Both share the speech model: Parakeet (the parakeet-ultra weights) in Utter's own ONNX
runtime.

## How it works

```
 Right Ctrl down                                                        Right Ctrl up
      │                                                                      │
      ▼                                                                      ▼
  mic (already open, 300 ms pre-roll) ──► voice detection cuts at pauses ──► last segment
                                              │                                │
                                              ▼                                ▼
                               Parakeet on the GPU, segment by segment ──► full transcript
                                              │                                │
                                              ▼                                ▼
                               the Bar shows the words live            cleanup model (llama.cpp)
                                                                               │
                                                                               ▼
                                      paste into the window that had focus, clipboard restored
```

- **One process, no hops.** Audio, the hotkey, speech recognition, insertion and storage run in
  the Rust core inside the app. The cleanup model is llama.cpp's server as a child process, kept
  warm, ended by Windows if Utter ever dies.
- **Most of the work happens while you talk.** Each pause sends the finished speech to the
  recognizer, so at key up only the last few words are left.
- **The GPU is kept awake.** An idle GPU drops its clocks and a cold pass is 5 times slower, so
  Utter warms it at key down and holds the memory clock up until key up.
- **Standalone.** Utter uses nothing installed on the machine but the GPU driver. onnxruntime,
  CUDA, cuDNN, llama.cpp and the models are pinned components with SHA-256 checksums
  ([components.json](crates/utter-runtime/components.json)), downloaded on first run and
  repaired in one click if a file is ever damaged.
- **The UI only renders.** The Svelte windows show state and send intent; every decision and
  every timer lives in Rust, where it can be tested without a window.

## How we build it

**Latency is the product.** Every stage of every dictation is traced. Nothing is called faster
or slower without a measurement, and a change that costs latency for a small gain in quality is
not made.

**Evals decide, not impressions.** Models are chosen by measurement on our own bench, never by
reputation. The cleanup model is judged on dictation built from real written text: everyday
messages, emails, notes and AI prompts, Stack Exchange answers, Wikipedia, arXiv abstracts and
chat logs, spoken in seven ways
(plain, with fillers, self-corrected, as commands, with far-back edits, with spoken punctuation,
in broken grammar), voiced by text-to-speech, and recognized by the same Parakeet the app runs.
The answer key is known by construction. Every change to a model, prompt or setting is run
against it and logged in [docs/eval-results.md](docs/eval-results.md), regressions included.

**Specs before code.** Every surface (the [Bar](docs/ux/bar.md), [first run](docs/ux/first-run.md),
[Home](docs/ux/home.md), [History](docs/ux/history.md), [insertion](docs/ux/insertion.md)) has a
spec with its layout, states, motion, copy, keyboard behavior and acceptance checks. It is done
when every check passes on the real machine.

**Decisions are written down.** [DESIGN.md](DESIGN.md) is the source of truth for scope and
architecture, and its decisions log records each choice with the measurement behind it.

**Done means done.** A feature ships when it meets its latency budget, its eval numbers and its
UX checks, not when it first works.

**Open data.** The training and test data for the cleanup models is published:

| Dataset | What is in it |
|---|---|
| [dictation-to-text](https://huggingface.co/datasets/ductai199x/dictation-to-text) | 89k pairs of what the recognizer heard and the text that was meant, 80k train / 10k test, balanced across sources, domains and kinds |
| [dictation-speech](https://huggingface.co/datasets/ductai199x/dictation-speech) | The spoken audio behind them (16 kHz FLAC), plus the eval sets, with voices the training data never uses |

Both carry only openly licensed text (CC0, CC-BY-SA, public domain), with the source and license
on every row.

## Repository

```
crates/
  utter-core        data folder, logs, tracing
  utter-runtime     pinned components: download, verify, install, repair
  utter-audio       microphone capture, ring buffer, resampling, levels
  utter-asr         Parakeet on onnxruntime, Silero voice detection, segmenting
  utter-llm         the llama.cpp server, the cleanup prompt and its guards
  utter-platform    everything OS-specific, behind traits: hotkey, insertion, focus, caret
  utter-pipeline    the dictation loop, settings, history
  utter-cli         benches and tools: transcribe, clean, dictate, e2e-bench, llm-bench
app/                Tauri 2 shell (src-tauri/) and the Svelte 5 windows (src/)
tools/              Python, never shipped: eval harness, datasets, training, benchmarks
docs/               UX specs, eval results, research notes, local setup
```

## Building from source

Rust (see `rust-version` in Cargo.toml), Node, and an NVIDIA GPU. From the repository root:

```bash
cargo run --release -p utter-cli -- setup
```

That installs the runtime components and models into the development data folder. Then, from
`app/`:

```bash
npx tauri dev
```

[docs/local-setup.md](docs/local-setup.md) has the details: where data lives, the Python tools
environment (uv), and every bench and eval command.

## Status

Windows, English. Next: context from the text around the cursor, styles per app, Command Mode,
then Linux (GNOME) and macOS. Vietnamese, with switching between the two mid-sentence, is
planned.

## License

Copyright (C) 2026 Tai Duc Nguyen.

Utter's code is licensed under the [GNU Affero General Public License v3.0](LICENSE): you may use,
study, change and share it, and if you distribute a modified version or run it as a service,
you must publish its source under the same license.

The models and datasets carry their own licenses, stated on their pages: the speech model's
weights are CC-BY-4.0, and the datasets are CC-BY-SA with the source and license of every row.
