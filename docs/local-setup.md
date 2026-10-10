# Local setup

How the development machine is laid out. Tai's choice (2026-10-09): downloads go on C:.

## Where things live

| What | Where | Notes |
|---|---|---|
| Models under evaluation (Python tools) | `%USERPROFILE%\.cache\huggingface\hub` | The standard Hugging Face cache, shared with other projects. Never committed. |
| Utter's runtime components (incl. shipped models) | `%USERPROFILE%\.cache\utter\runtime\<id>-<version>\` | Installed by `utter-cli setup` from `crates/utter-runtime/components.json`. |
| Eval clips and manifests | `%USERPROFILE%\.cache\utter\eval\<set>\` | 16 kHz mono WAVs plus `manifest.jsonl`. Override the root with `UTTER_DATA_DIR`. |
| Eval results | `%USERPROFILE%\.cache\utter\eval\results\` | One JSONL per model and set, re-scored by the report commands. |
| Local model exports | `%USERPROFILE%\.cache\utter\models\<name>\` | Output of `tools/export/export_parakeet.py`. The published copy is the `parakeet-ultra-onnx` component, from Hugging Face `ductai199x/utter-parakeet-ultra-onnx` (public, CC-BY-4.0). `utter-cli --model-dir` runs a local export instead. |
| Logs | `%USERPROFILE%\.cache\utter\logs\` | `llama-server.log` |

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
| parakeet-ultra-onnx 3aa6ab31 (2.6 GB) | Hugging Face `ductai199x/utter-parakeet-ultra-onnx` | The speech model |
| gemma4-26b-a4b-qat d1c082be (14.4 GB) | Hugging Face `google/gemma-4-26B-A4B-it-qat-q4_0-gguf` | The cleanup model |

`utter-cli setup` downloads, verifies and installs them, and the speech model (2.6 GB, from Hugging Face `ductai199x/utter-parakeet-ultra-onnx`), into
`%USERPROFILE%\.cache\utter\runtime\` (about 18.3 GB in all, most of it the two models); `utter-cli doctor` lists them. The same
path serves development and release; nothing is loaded from `tools/.venv`, LM Studio or any
other application. Checked by running with a PATH holding only Windows' own folders.

llama-server runs as Utter's child process and is tied to it with a Windows job object: if
Utter crashes or is killed, Windows ends the server too (checked by force-killing the CLI
mid-request). Its log is `%USERPROFILE%\.cache\utter\logs\llama-server.log`.

`UTTER_ORT_DYLIB` names a different onnxruntime library, for testing another build.

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
cargo run --release -p utter-cli -- bench      # the Rust runtime on the English sets
```

Benches of the Rust runtime are sensitive to everything else using the GPU; note the GPU's
utilization (`nvidia-smi`) next to any number you record.
