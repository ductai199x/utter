# Local setup

How the development machine is laid out. Tai's choice (2026-10-09): downloads go on C:.

## Where things live

| What | Where | Notes |
|---|---|---|
| Models (speech, LLM weights from Hugging Face) | `C:\Users\Tai\.cache\huggingface\hub` | The standard Hugging Face cache, shared with other projects. Never committed. |
| Eval clips and manifests | `C:\Users\Tai\.cache\utter\eval\<set>\` | 16 kHz mono WAVs plus `manifest.jsonl`. Override the root with `UTTER_DATA_DIR`. |
| Eval results | `C:\Users\Tai\.cache\utter\eval\results\` | One JSONL per model and set, re-scored by the report commands. |
| Exported models for the Rust core | `C:\Users\Tai\.cache\utter\models\<name>\` | `parakeet-ultra`: ONNX graphs, mel filters, vocabulary, config (`tools/export/export_parakeet.py`). |
| llama.cpp builds (when fetched) | `C:\Users\Tai\.cache\utter\llama.cpp\<build>\` | |

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

## The Rust core in development

`utter-asr` loads onnxruntime at run time. In development it finds `tools/.venv`'s
onnxruntime-gpu by itself (walking up from the current folder) and puts the nvidia wheels'
CUDA and cuDNN DLLs on the process's search path. `UTTER_ORT_DYLIB` overrides the library.
So `uv sync` in `tools/` is all the Rust side needs.

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
cargo run --release -p utter-cli -- transcribe some.wav
cargo run --release -p utter-cli -- bench      # the Rust runtime on the English sets
```

Benches of the Rust runtime are sensitive to everything else using the GPU; note the GPU's
utilization (`nvidia-smi`) next to any number you record.
