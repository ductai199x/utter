"""Fine-tunes the cleanup model: LoRA on Gemma 4 E4B (Google's unquantized QAT checkpoint, so
the result quantizes back to Q4_0 the way the shipped model was trained to), on pairs of
(what the recognizer heard, what the speaker meant to type).

Data: JSONL rows {"prompt": [system, user messages], "completion": [assistant message]} in the
exact chat format the product sends (crates/utter-llm/src/cleanup.rs, messages_with). Each row
is rendered with the model's own chat template; the loss is on the completion's tokens only.

Plain Transformers + PEFT, no TRL: TRL's trainer (1.15) always uses a Triton loss kernel, and
Triton ships for Linux only.

Output: <out>/adapter (the LoRA) and <out>/merged (full model, for convert_hf_to_gguf.py).

Run: UV_PROJECT_ENVIRONMENT=C:/Users/Tai/.cache/utter/envs/train uv run python train_lora.py DATA OUT
On N GPUs (Linux, e.g. FLAME): uv run torchrun --nproc-per-node N train_lora.py DATA OUT --batch B
(the effective batch is N x B x grad_accum).
"""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import typer

BASE = "google/gemma-4-e4b-it-qat-q4_0-unquantized"
BASE_REVISION = "476025a01dbf99361c062bbeca3d6a76bb4c4566"
# Attention and MLP projections of the language model only; the vision and audio towers stay
# untouched.
TARGETS = r".*language_model.*\.(q_proj|k_proj|v_proj|o_proj|gate_proj|up_proj|down_proj)"


def encode(tokenizer, row: dict, max_length: int) -> dict | None:
    """Token ids of the whole conversation and labels that count the completion only; None if
    it is longer than `max_length`."""
    prompt = tokenizer.apply_chat_template(
        row["prompt"], add_generation_prompt=True, tokenize=False
    )
    full = tokenizer.apply_chat_template(row["prompt"] + row["completion"], tokenize=False)
    if not full.startswith(prompt):
        raise ValueError(
            "the chat template does not render the prompt as a prefix of the conversation"
        )
    prompt_ids = tokenizer(prompt, add_special_tokens=False)["input_ids"]
    full_ids = tokenizer(full, add_special_tokens=False)["input_ids"]
    if len(full_ids) > max_length:
        return None
    labels = [-100] * len(prompt_ids) + full_ids[len(prompt_ids) :]
    return {"input_ids": full_ids, "labels": labels}


class Collator:
    """Pads a batch: input ids with the pad token, labels with -100 (ignored by the loss)."""

    def __init__(self, pad_id: int) -> None:
        self.pad_id = pad_id

    def __call__(self, batch: list[dict]):
        import torch

        n = max(len(b["input_ids"]) for b in batch)
        ids = torch.full((len(batch), n), self.pad_id, dtype=torch.long)
        labels = torch.full((len(batch), n), -100, dtype=torch.long)
        mask = torch.zeros((len(batch), n), dtype=torch.long)
        for i, b in enumerate(batch):
            k = len(b["input_ids"])
            ids[i, :k] = torch.tensor(b["input_ids"])
            labels[i, :k] = torch.tensor(b["labels"])
            mask[i, :k] = 1
        return {"input_ids": ids, "labels": labels, "attention_mask": mask}


def save_merged(model, tokenizer, merged_dir: Path) -> None:
    """Folds the LoRA into the weights and writes the full model where the converter reads it."""
    merged = model.merge_and_unload()
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


def _load(path: Path, tokenizer, max_length: int) -> list[dict]:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]
    out = [e for r in rows if (e := encode(tokenizer, r, max_length)) is not None]
    print(f"{path.name}: {len(out)} of {len(rows)} rows within {max_length} tokens")
    return out


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
    device: str = "cuda",
    max_steps: int = -1,
    merge: bool = typer.Option(
        True,
        help="Also write the merged model (for export_gguf.py); off when only the adapter "
        "is copied back (merge_adapter.py merges it there).",
    ),
) -> None:
    import torch
    from peft import LoraConfig, get_peft_model
    from transformers import AutoModelForImageTextToText, AutoTokenizer, Trainer, TrainingArguments

    tokenizer = AutoTokenizer.from_pretrained(BASE, revision=BASE_REVISION)
    train_rows = _load(data, tokenizer, max_length)
    eval_rows = _load(eval_data, tokenizer, max_length) if eval_data else None

    # Under torchrun (data parallel, one process per GPU) each process loads its copy straight
    # onto its own GPU, so host memory holds no full copy per process.
    local_rank = int(os.environ.get("LOCAL_RANK", "-1"))
    model = AutoModelForImageTextToText.from_pretrained(
        BASE,
        revision=BASE_REVISION,
        dtype=torch.bfloat16,
        device_map={"": f"cuda:{local_rank}"} if local_rank >= 0 else device,
        attn_implementation="sdpa",
    )
    model.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
    model.enable_input_require_grads()
    lora = LoraConfig(
        r=rank, lora_alpha=alpha, lora_dropout=0.05, target_modules=TARGETS, task_type="CAUSAL_LM"
    )
    model = get_peft_model(model, lora)
    model.print_trainable_parameters()

    args = TrainingArguments(
        output_dir=str(out / "checkpoints"),
        num_train_epochs=epochs,
        max_steps=max_steps,
        learning_rate=lr,
        lr_scheduler_type="cosine",
        warmup_steps=20,
        per_device_train_batch_size=batch,
        per_device_eval_batch_size=batch,
        gradient_accumulation_steps=grad_accum,
        bf16=device.startswith("cuda"),
        logging_steps=10,
        eval_strategy="steps" if eval_rows else "no",
        eval_steps=200,
        save_strategy="epoch",
        save_total_limit=2,
        report_to="none",
        seed=seed,
        remove_unused_columns=False,
        dataloader_num_workers=0,
        # Only the LoRA weights train, and all of them every step.
        ddp_find_unused_parameters=False,
    )
    trainer = Trainer(
        model=model,
        args=args,
        train_dataset=train_rows,
        eval_dataset=eval_rows,
        data_collator=Collator(tokenizer.pad_token_id),
    )
    trainer.train()
    if not trainer.is_world_process_zero():
        return

    model.save_pretrained(out / "adapter")
    if merge:
        save_merged(model, tokenizer, out / "merged")
    record = {
        "base": BASE,
        "revision": BASE_REVISION,
        "data": str(data),
        "rows": len(train_rows),
        "epochs": epochs,
        "lr": lr,
        "rank": rank,
        "alpha": alpha,
        "batch": batch,
        "grad_accum": grad_accum,
        "max_length": max_length,
        "seed": seed,
        "processes": trainer.args.world_size,
    }
    (out / "train_args.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    print(f"done -> {out}")


if __name__ == "__main__":
    typer.run(main)
