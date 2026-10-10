"""Merges a LoRA adapter trained elsewhere (e.g. on FLAME, by train_lora.py) into the base model
on this machine, so only the adapter has to be copied back. The result is the same <out>/merged
folder train_lora.py writes, ready for export_gguf.py. Runs on the CPU: the model is ~16 GB in
bf16, more than the local GPU holds alongside anything else.

Run: UV_PROJECT_ENVIRONMENT=C:/Users/Tai/.cache/utter/envs/train uv run python merge_adapter.py ADAPTER OUT
"""

from __future__ import annotations

from pathlib import Path

import typer
from train_lora import BASE, BASE_REVISION, save_merged


def main(adapter: Path, out: Path, device: str = "cpu") -> None:
    import torch
    from peft import PeftModel
    from transformers import AutoModelForImageTextToText, AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(BASE, revision=BASE_REVISION)
    model = AutoModelForImageTextToText.from_pretrained(
        BASE, revision=BASE_REVISION, dtype=torch.bfloat16, device_map=device
    )
    model = PeftModel.from_pretrained(model, adapter)
    save_merged(model, tokenizer, out)
    print(f"merged {adapter} into {out}")


if __name__ == "__main__":
    typer.run(main)
