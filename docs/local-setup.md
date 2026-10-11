# Local setup

How the development machine is laid out. Tai's choice (2026-10-09): downloads go on C:.

## Where things live

| What | Where | Notes |
|---|---|---|
| Models under evaluation (Python tools) | `C:\Users\Tai\.cache\huggingface\hub` | The standard Hugging Face cache, shared with other projects. Never committed. |
| Utter's runtime components (incl. shipped models) | `C:\Users\Tai\.cache\utter\runtime\<id>-<version>\` | Installed by `utter-cli setup` from `crates/utter-runtime/components.json`. |
| Eval clips and manifests | `C:\Users\Tai\.cache\utter\eval\<set>\` | 16 kHz mono WAVs plus `manifest.jsonl`. Override the root with `UTTER_DATA_DIR`. |
| Eval results | `C:\Users\Tai\.cache\utter\eval\results\` | One JSONL per model and set, re-scored by the report commands. |
| Local model exports | `C:\Users\Tai\.cache\utter\models\<name>\` | Output of `tools/export/export_parakeet.py`. The published copy is the `parakeet-ultra-onnx` component, from Hugging Face `ductai199x/utter-parakeet-ultra-onnx` (public, CC-BY-4.0). `utter-cli --model-dir` runs a local export instead. |
| Logs | `C:\Users\Tai\.cache\utter\logs\` | `llama-server.log` |
| Dictation log | `C:\Users\Tai\.cache\utter\dictations\` | One `<id>.json` per live dictation (every recognizer pass, Bar state, cleanup input and output, timings) and `<id>.wav`. Never leaves the machine. |

The Hugging Face account is `ductai199x`. Its terms were accepted for the gated repos
`speechcolab/gigaspeech2`, `linhtran92/viet_bud500` and `CohereLabs/cohere-transcribe-03-2026`.

## The tools environment

`tools/` is a uv project (Python 3.12). `uv sync` in `tools/` installs:
- the eval tooling (Hugging Face hub, pyarrow, soundfile, soxr, jiwer);
- `runtime`: onnxruntime-gpu with the CUDA 13 and cuDNN 9 DLLs from the nvidia wheels, plus
  onnx-asr, the Python reference for NeMo ONNX models;
- `models`: PyTorch (CUDA 13.0 build, from PyTorch's index) and Transformers, the official
  runtimes of the speech models under test.

Set `PYTHONUTF8=1` on Windows: Vietnamese text does not survive the console's default code
page.

Speech synthesis engines and model training each have their own uv project (`tools/tts/<engine>/`,
`tools/train/`), because they pin their own PyTorch and Transformers. Their environments live
on C: (D: is nearly full), selected per command:

```
cd tools/tts/kokoro
UV_PROJECT_ENVIRONMENT=C:/Users/Tai/.cache/utter/envs/tts-kokoro uv run python synth.py ...
```

The final-text eval (docs/plan-final-text.md) runs in order: `eval.finaltext.sources`
(passages), `eval.finaltext.spoken` (spoken forms), `eval.finaltext.screen` (gold quality),
`tools/tts/<engine>/synth.py` (audio), then `eval.finaltext.report asr`, `cleanup` and `score`.

## The Rust core: standalone

Utter depends on nothing installed on the machine except the GPU driver (Tai, 2026-10-09).
Everything else is a runtime component pinned in `crates/utter-runtime/components.json` to a
version, a publisher's URL and that publisher's SHA-256:

| Component | Source | What for |
|---|---|---|
| onnxruntime-gpu 1.31.0 | Microsoft's GitHub release | The speech model |
| cuda-runtime 13.4 (cudart, cuBLAS) | ggml-org's llama.cpp release | Both onnxruntime and llama.cpp |
| cudnn 9.27 | NVIDIA's PyPI package | onnxruntime's convolutions |
| llama-cpp b11535 (llama-server) | ggml-org's GitHub release | The language model, as Utter's child process |
| parakeet-ultra-onnx-fp16 3f7de939 (1.4 GB) | Hugging Face `ductai199x/utter-parakeet-ultra-onnx`, `fp16/` | The speech model (half-precision encoder) |
| utter-cleanup-e4b b0646215 (5.2 GB) | Hugging Face `ductai199x/utter-cleanup-e4b` | The cleanup model, light tier (default): Gemma 4 E4B fine-tuned (ft5) |
| gemma4-e4b-qat 4b4a2c1d (5.2 GB) | Hugging Face `google/gemma-4-E4B-it-qat-q4_0-gguf` | The prompted E4B, kept for evals: optional |
| gemma4-26b-a4b-qat d1c082be (14.4 GB) | Hugging Face `google/gemma-4-26B-A4B-it-qat-q4_0-gguf` | The cleanup model, quality tier: optional, `utter-cli setup gemma4-26b-a4b-qat` |

`utter-cli setup` downloads, verifies and installs them, and the speech model (2.6 GB, from Hugging Face `ductai199x/utter-parakeet-ultra-onnx`), into
`C:\Users\Tai\.cache\utter\runtime\` (about 8.2 GB for the default tier, most of it the two models); `utter-cli doctor` lists them. The same
path serves development and release; nothing is loaded from `tools/.venv`, LM Studio or any
other application. Checked by running with a PATH holding only Windows' own folders.

llama-server runs as Utter's child process and is tied to it with a Windows job object: if
Utter crashes or is killed, Windows ends the server too (checked by force-killing the CLI
mid-request). Its log is `C:\Users\Tai\.cache\utter\logs\llama-server.log`.

`UTTER_ORT_DYLIB` names a different onnxruntime library, for testing another build. It is
also how the recognizer runs on Linux before Linux components exist (FLAME, 2026-10-10;
transcripts identical to Windows on 100 of 100 clips):
- build: `cargo build --release -p utter-cli`. The microphone crate needs ALSA headers:
  without root, unpack `libasound2-dev` (`curl` the .deb matching the installed libasound2
  from archive.ubuntu.com, `dpkg -x`) into `~/sysroot`, link `libasound.so` to the system's
  `libasound.so.2`, and build with `PKG_CONFIG_SYSROOT_DIR=~/sysroot`
  `PKG_CONFIG_PATH=~/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig`;
- onnxruntime and CUDA: a venv with `onnxruntime-gpu==1.31.0` and the `nvidia-*-cu12` libraries;
  `UTTER_ORT_DYLIB=<venv>/.../onnxruntime/capi/libonnxruntime.so.1.31.0`,
  `LD_LIBRARY_PATH` = the `nvidia/*/lib` folders;
- models: copy the installed `parakeet-ultra-onnx-fp16-*` and `silero-vad-*` folders into
  `$UTTER_DATA_DIR/runtime/`.

## Commands

```
cd tools
uv run python -m eval.fetch_sets               # fetch every eval set (300 clips each)
uv run python -m eval.asr_bench run            # every Python candidate on every set it covers
uv run python -m eval.asr_bench report         # re-score what is on disk
uv run python -m eval.asr_bench compare A B    # paired bootstrap of two models' WER
uv run python -m export.export_parakeet        # export parakeet-ultra for the Rust core
uv run python -m export.check_parakeet         # check the export against Transformers

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

It loads the same components as `utter-cli dictate`; run `setup` first. Logs go to the
console in a debug build (`RUST_LOG=debug` for more).

To look at the Bar without the native app: `npm run dev` in `app/`, then open
`http://localhost:1430/bar.html?demo`, which plays a dictation through it (dev builds only).

Benches of the Rust runtime are sensitive to everything else using the GPU; note the GPU's
utilization (`nvidia-smi`) next to any number you record.
