"""Fine-tunes the cleanup model: LoRA on Gemma 4 E4B (Google's unquantized QAT checkpoint, so
the result quantizes back to Q4_0 the way the shipped model was trained to), on pairs of
(what the recognizer heard, what the speaker meant to type).

Data: JSONL rows {"prompt": [system, user messages], "completion": [assistant message]} in the
exact chat format the product sends (crates/utter-llm/src/cleanup.rs, messages_with). Loss is
on the completion only.

Output: <out>/adapter (the LoRA) and <out>/merged (full model, for convert_hf_to_gguf.py).

Run: UV_PROJECT_ENVIRONMENT=C:/Users/Tai/.cache/utter/envs/train uv run python train_lora.py DATA OUT
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import typer

BASE = "google/gemma-4-e4b-it-qat-q4_0-unquantized"
BASE_REVISION = "476025a01dbf99361c062bbeca3d6a76bb4c4566"
# Attention and MLP projections of the language model only; the vision and audio towers stay
# untouched.
TARGETS = r".*language_model.*\.(q_proj|k_proj|v_proj|o_proj|gate_proj|up_proj|down_proj)"


def main(
    data: Path,
    out: Path,
    eval_data: Path | None = None,
    epochs: float = 2.0,
    lr: float = 1e-4,
    rank: int = 16,
    alpha: int = 32,
    batch: int = 8,
    grad_accum: int = 2,
    max_length: int = 1024,
    seed: int = 0,
) -> None:
    import torch
    from datasets import load_dataset
    from peft import LoraConfig
    from transformers import AutoModelForImageTextToText, AutoTokenizer
    from trl import SFTConfig, SFTTrainer

    tokenizer = AutoTokenizer.from_pretrained(BASE, revision=BASE_REVISION)
    model = AutoModelForImageTextToText.from_pretrained(
        BASE,
        revision=BASE_REVISION,
        dtype=torch.bfloat16,
        device_map="cuda",
        attn_implementation="sdpa",
    )
    files = {"train": str(data)}
    if eval_data is not None:
        files["eval"] = str(eval_data)
    ds = load_dataset("json", data_files=files)

    cfg = SFTConfig(
        output_dir=str(out / "checkpoints"),
        num_train_epochs=epochs,
        learning_rate=lr,
        lr_scheduler_type="cosine",
        warmup_ratio=0.03,
        per_device_train_batch_size=batch,
        gradient_accumulation_steps=grad_accum,
        gradient_checkpointing=True,
        bf16=True,
        max_length=max_length,
        completion_only_loss=True,
        logging_steps=10,
        eval_strategy="steps" if eval_data is not None else "no",
        eval_steps=200,
        save_strategy="epoch",
        save_total_limit=2,
        report_to="none",
        seed=seed,
        dataloader_num_workers=0,
    )
    lora = LoraConfig(
        r=rank,
        lora_alpha=alpha,
        lora_dropout=0.05,
        target_modules=TARGETS,
        task_type="CAUSAL_LM",
    )
    trainer = SFTTrainer(
        model=model,
        args=cfg,
        train_dataset=ds["train"],
        eval_dataset=ds.get("eval"),
        processing_class=tokenizer,
        peft_config=lora,
    )
    trainer.train()

    adapter = out / "adapter"
    trainer.model.save_pretrained(adapter)
    merged = trainer.model.merge_and_unload()
    merged_dir = out / "merged"
    merged.save_pretrained(merged_dir, safe_serialization=True)
    tokenizer.save_pretrained(merged_dir)
    # The converter wants the processor and chat template files beside the weights.
    from huggingface_hub import snapshot_download

    src = Path(
        snapshot_download(BASE, revision=BASE_REVISION, allow_patterns=["*.json", "*.jinja"])
    )
    for f in src.iterdir():
        if f.suffix in (".json", ".jinja") and not (merged_dir / f.name).exists():
            shutil.copy(f, merged_dir / f.name)
    (out / "train_args.json").write_text(
        json.dumps(
            {
                "base": BASE,
                "revision": BASE_REVISION,
                "data": str(data),
                "epochs": epochs,
                "lr": lr,
                "rank": rank,
                "alpha": alpha,
                "batch": batch,
                "grad_accum": grad_accum,
                "max_length": max_length,
                "seed": seed,
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    print(f"done -> {out}")


if __name__ == "__main__":
    typer.run(main)
