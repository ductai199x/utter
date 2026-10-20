<p align="center">
  <img src="app/brand/icon.svg" width="88" alt="Utter">
</p>

<h1 align="center">Utter</h1>

<p align="center">
  <b>Hold a key, speak, and clean text appears in the app you are typing in.</b><br>
  Voice dictation that runs entirely on your own computer: fast, private, and open.
</p>

<p align="center">
  <a href="https://github.com/ductai199x/utter/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/ductai199x/utter?label=release&color=4a5cf0"></a>
  <a href="LICENSE"><img alt="License: AGPL-3.0" src="https://img.shields.io/badge/license-AGPL--3.0-4a5cf0"></a>
  <img alt="Windows and Linux" src="https://img.shields.io/badge/Windows%20%7C%20Linux-4a5cf0">
  <a href="https://huggingface.co/ductai199x/utter"><img alt="Models on Hugging Face" src="https://img.shields.io/badge/models-Hugging%20Face-4a5cf0"></a>
</p>

<p align="center">
  <a href="https://github.com/ductai199x/utter/releases/latest"><b>Download for Windows or Linux</b></a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="docs/eval-results.md">Eval results</a> ·
  <a href="DESIGN.md">Design</a>
</p>

<p align="center">
  <img src="docs/media/demo.gif" width="880" alt="Holding Right Ctrl and saying 'hey sarah um yes I can take the release review on thursday no wait friday at ten'. The Bar at the bottom of the screen shows the words as they are heard, the correction is applied, and on release 'Hey Sarah, yes, I can take the release review on Friday at 10.' appears in the chat box.">
</p>

Talk the way you talk. Say "um", change your mind halfway through a sentence, list things off.
Utter types what you meant, punctuated and corrected, into whatever has the cursor: email, chat,
an editor, a terminal, an AI prompt. The text is there about a quarter of a second after you let
go, and nothing you say ever leaves your machine.

<table>
  <tr>
    <td width="50%" valign="top">
      <b>Private by construction</b><br>
      Speech recognition and cleanup run on your graphics card. No account, no cloud, no
      telemetry: Utter goes online only to download the models it runs on.
    </td>
    <td width="50%" valign="top">
      <b>Fast enough to forget about</b><br>
      231-260 ms from releasing the key to text in your app, measured in the app itself. Most
      of the work happens while you are still talking.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <b>Writes like you meant it</b><br>
      Fillers go, punctuation comes in, "three, no, four" becomes 4, spoken lists become bullets,
      and your names and terms are spelled your way.
    </td>
    <td width="50%" valign="top">
      <b>Open all the way down</b><br>
      AGPL code, published models, and published training and test data, every row with its
      source and license.
    </td>
  </tr>
</table>

## A quick tour

<table>
  <tr>
    <td width="50%" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/history-dark.png">
        <img src="docs/media/history-light.png" alt="History: every dictation, searchable, with what Utter heard and what it changed">
      </picture>
      <p><b>History.</b> Every dictation, searchable by what was typed or what was heard, with
      cleanup's changes marked word by word. On your computer only, kept as long as you choose.</p>
    </td>
    <td width="50%" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/rules-dark.png">
        <img src="docs/media/rules-light.png" alt="Paste rules: how each app pastes, with a one-click fix for an app where the last paste did not land">
      </picture>
      <p><b>Paste rules.</b> Terminals, Office and remote desktops each paste their own way,
      and Utter knows them. When a paste does not land, Utter notices, keeps the text on the
      clipboard, and offers the fix for that app in one click.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/setup-dark.png">
        <img src="docs/media/setup-light.png" alt="First run: choose a model, with the one that fits your graphics card recommended">
      </picture>
      <p><b>First run.</b> Utter checks your computer, recommends the model that fits your
      graphics card, tests your microphone, downloads what it needs, and ends with a test
      dictation.</p>
    </td>
    <td width="50%" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/shortcuts-dark.png">
        <img src="docs/media/shortcuts-light.png" alt="Shortcuts: choose the key you hold to dictate">
      </picture>
      <p><b>Your key.</b> Hold Right Ctrl, or the key of your choice, to dictate; double-tap for
      hands-free; Esc cancels. Alt+Shift+Z pastes the last dictation again.</p>
    </td>
  </tr>
</table>

## Measured, not claimed

<img src="docs/media/latency.svg" width="880" alt="After the key goes up: the last words are recognized in 48-71 ms, the dictation is cleaned in 131-158 ms and pasted in 47-61 ms, 231-260 ms in all.">

On the development machine (RTX 3090 Ti), in the app itself. Every figure comes from
[docs/eval-results.md](docs/eval-results.md), where the method and the raw numbers live.

| | |
|---|---|
| Key up to text in the app | **231-260 ms** |
| Launch to ready | **3.5-3.8 s** warm, 5.6-6.0 s right after a reboot |
| Ready to send as typed, realistic dictation (Quality) | **88.1%**, 80.1% with voices the model never heard |
| Dictionary terms spelled right | **97.8%** |
| GPU memory (NVIDIA) | Quality **4.8 GB**, Light **3.2 GB** |

"Ready to send" is the share of dictations a careful writer would send exactly as typed, judged
dictation by dictation against the intended text with a rubric checked against human review.

## What it does

- **Dictation that reads like writing.** Fillers go, punctuation and capitals come in,
  self-corrections ("three, no, four") are applied, spoken lists become bullets, spoken
  punctuation ("comma", "new line") is typed as marks.
- **Typed, never answered.** Dictate a question, an instruction to an AI assistant, or text
  that tells the app to "ignore everything before": it is typed as you said it, never obeyed.
  A check in the code types the transcript itself if the model's output ever strays from it.
- **Your words, spelled your way.** A dictionary of names and terms steers both recognition
  and cleanup.
- **Never loses a dictation.** If the text cannot go in (the focus moved, an admin window, an
  app that ignores the paste), it stays on the clipboard and the Bar says so. Alt+Shift+Z
  pastes the last dictation again, and Retry runs the last one through again after an error.
  Your own clipboard is always put back.
- **Keeps working when things change.** Unplug the microphone or switch the Windows default
  and Utter follows; if the cleanup model ever crashes, it restarts on its own; model updates
  download while you keep dictating on the current version.
- **Fits the app.** Paste rules per app, automatic spacing when a dictation continues text,
  hands-free mode with a double-tap, Esc to cancel.

## How it works

```mermaid
flowchart LR
    key(["Hold the key"]) --> mic["Microphone<br/>always open, last 10 s kept"]
    mic --> vad["Voice detection<br/>cuts at each pause"]
    vad --> asr["Parakeet on the GPU<br/>each segment as you speak"]
    asr --> llm["Cleanup model<br/>fine-tuned Gemma, llama.cpp"]
    llm --> paste(["Pasted where the cursor is<br/>clipboard put back"])
    asr -. "words as heard" .-> bar["The Bar"]
    llm -. "cleaned text" .-> bar
```

- **Most of the work happens while you talk.** Each pause sends the finished speech to the
  recognizer and the cleanup model, so at key up only the last few words are left.
- **One process, no hops.** Audio, the hotkey, speech recognition, insertion and storage run in
  the Rust core inside the app. The cleanup model is llama.cpp's server as a supervised child
  process, kept warm and restarted if it ever dies.
- **The GPU is kept awake.** An idle GPU drops its clocks and a cold pass is five times slower,
  so Utter warms it at key down and holds it up until key up (on NVIDIA for now).
- **Standalone.** Utter uses nothing installed on the machine but the GPU driver. onnxruntime,
  CUDA and cuDNN on NVIDIA (onnxruntime's WebGPU provider on AMD and Intel), llama.cpp and the
  models are pinned components with SHA-256 checksums
  ([components.json](crates/utter-runtime/components.json)), downloaded on first run and
  repaired in one click if a file is ever damaged.
- **The UI only renders.** The Svelte windows show state and send intent; every decision and
  every timer lives in Rust, where it can be tested without a window.

## How we build it

**Latency is the product.** Every stage of every dictation is traced. Nothing is called faster
or slower without a measurement, and a change that costs latency for a small gain in quality is
not made.

**Evals decide, not impressions.** The cleanup model is judged on dictation built from real
written text (everyday messages, emails and AI prompts, Stack Exchange, Wikipedia, arXiv,
chat logs), spoken in seven ways (plain, with fillers, self-corrected, as commands, with
far-back edits, with spoken punctuation, in broken grammar), voiced by text-to-speech and
recognized by the same Parakeet the app runs. The answer key is known by construction. Every
change to a model, prompt or setting is run against it and logged in
[docs/eval-results.md](docs/eval-results.md), regressions included.

**Reproducible from public data.** The models Utter ships are trained by one recipe on the
published dataset, so anyone can rebuild them:

| | What it is |
|---|---|
| [dictation-to-text](https://huggingface.co/datasets/ductai199x/dictation-to-text) | 89k pairs of what the recognizer heard and the text that was meant, split by passage, balanced across sources, domains and kinds |
| [dictation-speech](https://huggingface.co/datasets/ductai199x/dictation-speech) | The spoken audio behind them (16 kHz FLAC), plus eval sets in voices the training data never uses |
| [utter](https://huggingface.co/ductai199x/utter) | The cleanup models: **Quality** (Gemma 4 E4B) and **Light** (Gemma 4 E2B), fine-tuned, Q4_0 GGUF, with their LoRA adapters |

**Specs before code.** Every surface (the [Bar](docs/ux/bar.md), [first run](docs/ux/first-run.md),
[Home](docs/ux/home.md), [History](docs/ux/history.md), [insertion](docs/ux/insertion.md)) has a
spec with its layout, states, motion, copy, keyboard behavior and acceptance checks, and is done
when every check passes on the real machine. [DESIGN.md](DESIGN.md) is the source of truth for
scope and architecture, with a decisions log that records each choice and the measurement
behind it.

## Install

From the [latest release](https://github.com/ductai199x/utter/releases/latest):

- **Windows 10 or 11 (64-bit):** run `Utter_<version>_x64-setup.exe`. It installs for your user
  only, and an update installs over it with your models and settings kept.
- **Ubuntu 24.04+, Debian 13, Mint 22:** `sudo apt install ./Utter_<version>_amd64.deb`.
- **Other Linux (Fedora 40+ and others):** make `Utter_<version>_amd64.AppImage` executable and
  open it. Utter asks once for keyboard access, so it can hear the dictation key.
- **macOS 14+ on Apple Silicon (M1 and later):** through Homebrew, then open Utter from
  Applications. It asks for Accessibility and the microphone once; updates keep both.

  ```bash
  brew tap ductai199x/utter https://github.com/ductai199x/utter
  brew install --cask utter
  ```

**Needs:** about 10 GB of disk, and one of:

- an NVIDIA graphics card with 4 GB of memory (6 GB for Quality) and driver 580 or newer;
- an AMD or Intel graphics card through Vulkan, as a preview: 6 GB of memory (8 GB for Quality),
  measured so far only on an NVIDIA card made to run the Vulkan path;
- no graphics card: the processor, with Light and Extra light (8 GB of memory);
- a Mac with Apple Silicon: its GPU runs both models (measured on an M2 with 16 GB; 8 GB Macs
  not yet).

Settings > Runs on switches between them.

## Building from source

Rust (see `rust-version` in Cargo.toml), Node, and a graphics card (NVIDIA, or AMD and Intel
through Vulkan). From the repository root:

```bash
cargo run --release -p utter-cli -- setup
```

That installs the runtime components and models into the development data folder. Then, from
`app/`:

```bash
npx tauri dev
```

[docs/local-setup.md](docs/local-setup.md) has the details: where data lives, the Python tools
(`utter-eval`, `utter-train`, `utter-data`, `utter-bench`, `utter-speech`) and every bench
command.

<details>
<summary>Repository layout</summary>

```
crates/
  utter-core        data folder, logs, tracing
  utter-runtime     pinned components: download, verify, install, repair, updates
  utter-audio       microphone capture, 10 s history, resampling, levels
  utter-asr         Parakeet on onnxruntime, Silero voice detection, segmenting
  utter-llm         the llama.cpp server, the cleanup prompt and its guards
  utter-platform    everything OS-specific, behind traits: hotkey, insertion, focus, caret
  utter-pipeline    the dictation loop, settings, history
  utter-cli         benches and tools: transcribe, clean, dictate, e2e-bench, llm-bench
app/                Tauri 2 shell (src-tauri/) and the Svelte 5 windows (src/)
tools/              Python, never shipped: eval, training, datasets, benchmarks, media
docs/               UX specs, eval results, research notes, local setup
```

</details>

## Status

Windows and Linux (GNOME and KDE), English; NVIDIA graphics, the processor, and AMD and Intel
graphics through Vulkan in preview until they are measured on real AMD and Intel cards; macOS on
Apple Silicon, tested on a Mac mini M2 with macOS 26. Next: Android. After that: context from
the text around the cursor, styles per app, Command Mode, and Vietnamese with switching between
the two mid-sentence.

## License

Copyright (C) 2026 Tai Duc Nguyen.

Utter's code is licensed under the [GNU Affero General Public License v3.0](LICENSE): you may
use, study, change and share it, and if you distribute a modified version or run it as a
service, you must publish its source under the same license.

The models and datasets carry their own licenses, stated on their pages: the speech model's
weights are CC-BY-4.0, and the datasets are CC-BY-SA with the source and license of every row.
