# Local setup

How the development machine is laid out. Tai's choice (2026-10-09): downloads go on C:.

## Where things live

| What | Where | Notes |
|---|---|---|
| Models under evaluation (Python tools) | `%USERPROFILE%\.cache\huggingface\hub` | The standard Hugging Face cache, shared with other projects. Never committed. |
| Utter's runtime components (incl. shipped models) | `%LOCALAPPDATA%\Utter_dev\runtime\<id>-<version>\` | Installed by `utter-cli setup` from `crates/utter-runtime/components.json`. |
| Eval sets | `%LOCALAPPDATA%\Utter_dev\eval\<set>\` | 16 kHz mono WAVs plus `manifest.jsonl` (`utter-eval` makes them from the datasets' test splits; `utter-bench fetch` the public ones). Override the root with `UTTER_DATA_DIR`. |
| Eval results | `%LOCALAPPDATA%\Utter_dev\eval\results\` | `asr/`, `llm/`, `score/`, `judge/` per run and set, and the comparison reports. |
| Data batches | `%LOCALAPPDATA%\Utter_dev\data\<batch>\` | `utter-data`'s working files for a batch of new data, until it is published. |
| Training runs | `%LOCALAPPDATA%\Utter_dev\train\<run>\` | `utter-train`: the data it trained on (with its dataset revision) and the adapter and merged model. |
| Local model exports | `%LOCALAPPDATA%\Utter_dev\models\<name>\` | Output of `utter-speech export`. The published copy is the `parakeet-ultra-onnx` component, from Hugging Face `ductai199x/utter-parakeet-ultra-onnx` (public, CC-BY-4.0). `utter-cli --model-dir` runs a local export instead. |
| Settings | `%LOCALAPPDATA%\Utter_dev\settings.toml` | The dictionary, list formatting, microphone and cleanup model. The Home window edits it; a hand edit applies on the next dictation too. Written with every key commented when missing. |
| Logs | `%LOCALAPPDATA%\Utter_dev\logs\` | `utter.log` and `llama-server.log`, rotated at 5 MB (three kept); the setting `keep_logs` turns them off. |
| Dictation log | `%LOCALAPPDATA%\Utter_dev\dictations\` | One `<id>.json` per live dictation (every recognizer pass, Bar state, cleanup input and output, timings) and `<id>.wav`. Never leaves the machine. |

The Hugging Face account is `ductai199x`. Its terms were accepted for the gated repos
`speechcolab/gigaspeech2`, `linhtran92/viet_bud500` and `CohereLabs/cohere-transcribe-03-2026`.

## The tools

`tools/` is a uv project (Python 3.12): `uv sync` in `tools/` installs it with five commands, one
per job, each with `--help`:

| Command | Does |
|---|---|
| `utter-data` | grows the published datasets, a batch at a time: passages, spoken versions, speech, transcripts, text rows, then build, check and upload |
| `utter-train` | fine-tunes a cleanup model on the text dataset (at a pinned revision, with the shipped recipe) and exports its Q4_0 GGUF |
| `utter-eval` | evaluates cleanup models on the datasets' test splits: how often each types what the speaker meant, with Gemma 4 26B as the judge |
| `utter-bench` | speech models on public test sets, by word error rate |
| `utter-speech` | rebuilds the speech model Utter ships: ONNX export, 8-bit encoder, check against Transformers |

The two datasets on the Hub are the standard every command reads and writes
(`tools/utter_tools/schema.py`). The environment holds:
- the tools themselves (Hugging Face hub, pyarrow, soundfile, soxr, jiwer, rapidfuzz, typer);
- `runtime`: onnxruntime-gpu with the CUDA 13 and cuDNN 9 DLLs from the nvidia wheels, plus
  onnx-asr, the Python reference for NeMo ONNX models;
- `models`: PyTorch (CUDA 13.0 build, from PyTorch's index) and Transformers, the official
  runtimes of the speech models the bench compares, and ONNX for the export.

Set `PYTHONUTF8=1` on Windows: Vietnamese text does not survive the console's default code
page.

Speech synthesis and training have their own uv projects under `tools/envs/` (`tts-kokoro`,
`tts-chatterbox`, `train`), because they pin their own PyTorch and Transformers; the commands
above start them (`uv run --project tools/envs/<name>`). Their environments live on C: (D: is
nearly full): set `UV_PROJECT_ENVIRONMENT` per project when running one by hand.

On Linux (FLAME), where Utter has no runtime components yet: `UTTER_CLI` points the eval at a
Linux build of `utter-cli`, `UTTER_LLAMA_SERVER` at a llama-server built at the runtime's tag
(b11535), and `UTTER_LLAMA_BUILD` the export at that llama.cpp tree.

The README's pictures (docs/media) come from the app's own pages: with `npm run dev` serving
them, `uv run --with playwright --with pillow python tools/media/record.py` drives the installed
Edge, records app/promo.html (the real Bar playing one dictation) into demo.gif and takes the
windows' screenshots on the pages' stand-in data, light and dark.

## Commands

```
cd tools
uv run utter-eval --model quality-v2=PATH.gguf --model quality-v1=PATH.gguf   # the cleanup eval
uv run utter-bench fetch                       # the public speech test sets (300 clips each)
uv run utter-bench run                         # every speech model on every set it covers
uv run utter-train --name quality-v3 --base e4b --gpus 2
uv run utter-data new my-batch --collection corpus-v3 --seed 7
uv run utter-speech export                     # export parakeet-ultra for the Rust core

cd ..
cargo test --workspace --release
cargo run --release -p utter-cli -- setup      # install the runtime components (once)
cargo run --release -p utter-cli -- doctor     # what is installed
cargo run --release -p utter-cli -- transcribe some.wav
cargo run --release -p utter-cli -- chat --gguf model.gguf "message"
cargo run --release -p utter-cli -- clean "um, so, the text"   # the cleanup model on its own
cargo run --release -p utter-cli -- record out.wav           # the microphone, 16 kHz, with pre-roll
cargo run --release -p utter-cli -- vad some.wav             # where voice detection cuts
cargo run --release -p utter-cli -- dictate                  # live: hold Right Ctrl, speak, release
cargo run --release -p utter-cli -- e2e-bench                # key up -> text ready, on eval clips
cargo run --release -p utter-cli -- bench      # the Rust runtime on the English sets
```

The app (the Bar overlay and the tray) needs Node 22 or newer for its web side:

```
cd app
npm ci
npx tauri dev                                  # the app, with the Bar hot-reloading
npx tauri build --no-bundle                    # target/release/utter.exe
npm run check                                  # svelte-check
```

**Linux** (Ubuntu 24.04, the build machine of the packages: their glibc is 2.39, so they run
on Ubuntu 24.04+, Fedora 40+ and Debian 13): the system libraries first, then the .deb and the
AppImage into `target/release/bundle`:

```
sudo apt install build-essential curl libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev
sh packaging/linux/build.sh
```

The hotkey and the paste keys need `packaging/linux/70-utter.rules` (the .deb installs it; the
app's first run offers to, through the desktop's password dialog). A machine without a usable
GPU (a VM) runs everything on the CPU with `UTTER_GPU_BACKEND=cpu`: slow, right for the desktop
side only.

**macOS** is checked by CI (`.github/workflows/ci.yml`) and not yet run: no Mac here.

It loads the same components as `utter-cli dictate`; run `setup` first. Logs go to the
console in a debug build (`RUST_LOG=debug` for more). Development builds are optimized
(`[profile.dev]` in Cargo.toml), so timings in `tauri dev` are the product's; a full rebuild
takes about 3 minutes, a change to Utter's crates 20-40 s.

**The whole app without speaking:** `UTTER_TEST_AUDIO=<16 kHz mono WAV>` (development builds
only) makes every dictation hear that file instead of the microphone, in real time, and lets
an injected Right Ctrl start one, so a script can hold the key (SendInput) and check the
paste, History and the Bar. Nothing is played. With `UTTER_DATA_DIR` pointing at a folder of
its own it never touches the real History or settings.

**The published datasets** (public on the Hub): `ductai199x/dictation-to-text`, the cleanup
models' training and test pairs, and `ductai199x/dictation-speech`, the speech behind them and the
eval sets. They grow only through `utter-data`: a batch is built against the Hub's current
revisions, audited row by row, and uploaded only after a clean audit and an explicit `--yes`.

**Run Utter from a normal terminal, not from inside a packaged app.** The Claude desktop app
is an MSIX package: a folder that a process started from it creates under `%LOCALAPPDATA%`
lands in the package's private store (`AppData\Local\Packages\<package>\LocalCache\Local\`),
which only that package sees. On 2026-10-12 the whole `Utter_dev` folder had been created
that way, so `npx tauri dev` in a normal terminal found it empty and downloaded everything.

To look at the Bar without the native app: `npm run dev` in `app/`, then open
`http://localhost:1430/bar.html?demo`, which plays a dictation through it (dev builds only).

Benches of the Rust runtime are sensitive to everything else using the GPU; note the GPU's
utilization (`nvidia-smi`) next to any number you record.
