"""Merges a LoRA adapter trained elsewhere (e.g. on a GPU cluster, by train_lora.py) into the base model
on this machine, so only the adapter has to be copied back. The result is the same <out>/merged
folder train_lora.py writes, ready for export_gguf.py. Runs on the CPU: the model is ~16 GB in
bf16, more than the local GPU holds alongside anything else.

The base model is the one the run recorded (train_args.json beside the adapter), so an adapter
is never merged into a model it was not trained on.

Run: UV_PROJECT_ENVIRONMENT=%USERPROFILE%/.cache/utter/envs/train uv run python merge_adapter.py ADAPTER OUT
"""

from __future__ import annotations

import json
from pathlib import Path

import typer
from train_lora import BASES, save_merged


def main(adapter: Path, out: Path, device: str = "cpu") -> None:
    import torch
    from peft import PeftModel
    from transformers import AutoModelForImageTextToText, AutoTokenizer

    args = adapter / "train_args.json"
    # Runs before train_args.json recorded the base were all trained from E4B.
    record = json.loads(args.read_text(encoding="utf-8")) if args.exists() else {}
    repo = record.get("base", BASES["e4b"][0])
    revision = record.get("revision", BASES["e4b"][1])
    print(f"base: {repo} @ {revision}")
    tokenizer = AutoTokenizer.from_pretrained(repo, revision=revision)
    model = AutoModelForImageTextToText.from_pretrained(
        repo, revision=revision, dtype=torch.bfloat16, device_map=device
    )
    model = PeftModel.from_pretrained(model, adapter)
    save_merged(model, tokenizer, out, repo, revision)
    print(f"merged {adapter} into {out}")


if __name__ == "__main__":
    typer.run(main)
