"""Fine-tunes the cleanup model: LoRA on a Gemma model (Google's unquantized QAT checkpoint, so
the result quantizes back to Q4_0 the way the shipped model was trained to), on pairs of
(what the recognizer heard, what the speaker meant to type).

Data: JSONL rows {"prompt": [system, user messages], "completion": [assistant message]} in the
exact chat format the product sends (crates/utter-llm/src/cleanup.rs, messages_with). Each row
is rendered with the model's own chat template; the loss is on the completion's tokens only.

Plain Transformers + PEFT, no TRL: TRL's trainer (1.15) always uses a Triton loss kernel, and
Triton ships for Linux only.

Output: <out>/adapter (the LoRA) and <out>/merged (the full model, for export_gguf.py).

Its defaults are the recipe Quality and Light are trained with (docs/eval-results.md); utter-train
run starts it:  uv run --project envs/train torchrun --nproc-per-node N train_lora.py DATA OUT
(the effective batch is N x batch x grad_accum).
"""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import typer

# The models a cleanup model is trained from, pinned: E4B for Quality, E2B for Light (its GPU part
# is half of E4B's: docs/eval-results.md), and Gemma 3's text-only 1B and 270M for the phone
# (docs/plan-android.md). All are Google's unquantized QAT checkpoints.
BASES = {
    "e4b": (
        "google/gemma-4-e4b-it-qat-q4_0-unquantized",
        "476025a01dbf99361c062bbeca3d6a76bb4c4566",
    ),
    "e2b": (
        "google/gemma-4-E2B-it-qat-q4_0-unquantized",
        "6befbaca7398925921802abd1f277b495b78b738",
    ),
    "g3-1b": (
        "google/gemma-3-1b-it-qat-q4_0-unquantized",
        "a6692c1945954f4aa39a17b8dfba4a7e62db3d4f",
    ),
    "g3-270m": (
        "google/gemma-3-270m-it-qat-q4_0-unquantized",
        "8f726c6a497fd439f0d6f726e52f8e3b439a26e5",
    ),
}
# Gemma 3's 1B and 270M are text-only models; Gemma 4's carry vision and audio towers.
TEXT_ONLY = {"g3-1b", "g3-270m"}

# Attention and MLP projections of the language model only; Gemma 4's vision and audio towers
# stay untouched.
PROJECTIONS = r"\.(q_proj|k_proj|v_proj|o_proj|gate_proj|up_proj|down_proj)"
TARGETS = r".*language_model.*" + PROJECTIONS
TEXT_TARGETS = r".*layers.*" + PROJECTIONS


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


def save_merged(model, tokenizer, merged_dir: Path, base: str, revision: str) -> None:
    """Writes the full model where the converter reads it, a LoRA folded into the weights first
    (a fully fine-tuned model is written as it is); `base` at `revision` is the model it was
    trained from."""
    merged = model.merge_and_unload() if hasattr(model, "merge_and_unload") else model
    merged.save_pretrained(merged_dir, safe_serialization=True)
    tokenizer.save_pretrained(merged_dir)
    # The converter wants the processor and chat template files beside the weights, and Gemma 3's
    # SentencePiece model (tokenizer.model), from which it reads the vocabulary.
    from huggingface_hub import snapshot_download

    src = Path(
        snapshot_download(
            base, revision=revision, allow_patterns=["*.json", "*.jinja", "tokenizer.model"]
        )
    )
    for f in src.iterdir():
        if f.suffix in (".json", ".jinja", ".model") and not (merged_dir / f.name).exists():
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
    epochs: float = 1.0,
    lr: float = 2e-4,
    rank: int = 16,
    alpha: int = 32,
    batch: int = 8,
    grad_accum: int = 1,
    max_length: int = 1024,
    seed: int = 0,
    base: str = typer.Option("e4b", help=f"The model to train from: {', '.join(BASES)}."),
    full: bool = typer.Option(
        False,
        help="Train every weight instead of a LoRA (rank and alpha unused): what a small model "
        "needs, as a LoRA leaves its pretrained habits in place (docs/plan-android.md).",
    ),
) -> None:
    import torch
    from peft import LoraConfig, get_peft_model
    from transformers import (
        AutoModelForCausalLM,
        AutoModelForImageTextToText,
        AutoTokenizer,
        Trainer,
        TrainingArguments,
    )

    repo, revision = BASES[base]
    text_only = base in TEXT_ONLY
    tokenizer = AutoTokenizer.from_pretrained(repo, revision=revision)
    train_rows = _load(data, tokenizer, max_length)
    eval_rows = _load(eval_data, tokenizer, max_length) if eval_data else None

    # Under torchrun (data parallel, one process per GPU) each process loads its copy straight
    # onto its own GPU, so host memory holds no full copy per process.
    local_rank = int(os.environ.get("LOCAL_RANK", "-1"))
    model_class = AutoModelForCausalLM if text_only else AutoModelForImageTextToText
    model = model_class.from_pretrained(
        repo,
        revision=revision,
        # Full training keeps float32 weights so small updates are not rounded away; the math
        # still runs in bfloat16 (bf16=True below).
        dtype=torch.float32 if full else torch.bfloat16,
        device_map={"": f"cuda:{local_rank}"} if local_rank >= 0 else "cuda",
        attn_implementation="sdpa",
    )
    model.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
    if not full:
        model.enable_input_require_grads()
        lora = LoraConfig(
            r=rank,
            lora_alpha=alpha,
            lora_dropout=0.05,
            target_modules=TEXT_TARGETS if text_only else TARGETS,
            task_type="CAUSAL_LM",
        )
        model = get_peft_model(model, lora)
        model.print_trainable_parameters()

    args = TrainingArguments(
        output_dir=str(out / "checkpoints"),
        num_train_epochs=epochs,
        learning_rate=lr,
        lr_scheduler_type="cosine",
        warmup_steps=20,
        per_device_train_batch_size=batch,
        per_device_eval_batch_size=batch,
        gradient_accumulation_steps=grad_accum,
        bf16=True,
        logging_steps=10,
        eval_strategy="steps" if eval_rows else "no",
        eval_steps=200,
        save_strategy="epoch",
        save_total_limit=2,
        report_to="none",
        seed=seed,
        remove_unused_columns=False,
        dataloader_num_workers=0,
        # The trained weights (the LoRA's, or all of them) all get a gradient every step.
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

    if not full:
        model.save_pretrained(out / "adapter")
    save_merged(model, tokenizer, out / "merged", repo, revision)
    record = {
        "base": repo,
        "revision": revision,
        "data": str(data),
        "rows": len(train_rows),
        "epochs": epochs,
        "lr": lr,
        "full": full,
        "rank": None if full else rank,
        "alpha": None if full else alpha,
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
