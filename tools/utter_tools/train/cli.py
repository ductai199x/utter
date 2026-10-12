"""utter-train: fine-tune a cleanup model on the text dataset and export the GGUF the app loads.

  utter-train run NAME --base e4b|e2b [--gpus N] [--ref REVISION]
      the dataset's training split at one revision, then LoRA with the recipe Quality and Light
      are trained with (envs/train/train_lora.py's defaults), in <data dir>/train/NAME/;
  utter-train export MERGED OUT.gguf
      Q4_0 GGUF with llama.cpp at the runtime's build, and its components.json size and SHA-256;
  utter-train pin REPO REVISION     components.json entries for files published on the Hub.

The training scripts run in their own environment (envs/train: PyTorch and Transformers pinned
apart from these tools).
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path

import typer

from utter_tools import config, hub

app = typer.Typer(
    add_completion=False, no_args_is_help=True, help="Fine-tune a cleanup model and export it."
)

ENV = config.REPO / "tools" / "envs" / "train"
# Shipped models are named utter-<tier>-v<N>-<quantization>.gguf (DESIGN.md decisions).
MODEL_NAME = re.compile(r"utter-(quality|light)-v\d+-q4_0\.gguf")


def _run(args: list[str]) -> None:
    subprocess.run(["uv", "run", "--project", str(ENV), *args], check=True, cwd=ENV)


def write_data(out: Path, ref: str | None) -> dict:
    """The text dataset's splits at `ref` as the trainer's JSONL ({"prompt": [system, user],
    "completion": [assistant]}), with where they came from (source.json)."""
    rev = hub.revision(config.TEXT_REPO, ref)
    out.mkdir(parents=True, exist_ok=True)
    record: dict = {"dataset": config.TEXT_REPO, "revision": rev, "files": {}}
    for split, name in (("train", "pairs.jsonl"), ("test", "pairs.eval.jsonl")):
        rows = hub.text_rows(config.TEXT_REPO, rev, split, ["id", "category", "messages"])
        with (out / name).open("w", encoding="utf-8", newline="\n") as fh:
            for r in rows:
                m = r["messages"]
                if [x["role"] for x in m] != ["system", "user", "assistant"]:
                    raise ValueError(f"{r['id']}: messages are not system, user, assistant")
                row = {"id": r["id"], "kind": r["category"], "prompt": m[:2], "completion": m[2:]}
                fh.write(json.dumps(row, ensure_ascii=False) + "\n")
        record["files"][name] = {"split": split, "rows": len(rows)}
    (out / "source.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    return record


@app.command()
def run(
    name: str,
    base: str = typer.Option(..., help="e4b (Quality) or e2b (Light)."),
    gpus: int = typer.Option(1, help="GPUs on this machine (torchrun processes)."),
    ref: str = typer.Option(None, help="dictation-to-text revision (default: main)."),
) -> None:
    """Train a cleanup model with the shipped recipe."""
    root = config.data_dir() / "train" / name
    if (root / "run").exists():
        raise typer.BadParameter(f"{root / 'run'} exists: choose another name")
    record = write_data(root / "data", ref)
    typer.echo(f"data: {config.TEXT_REPO} @ {record['revision'][:10]} {record['files']}")
    args = ["torchrun", "--nproc-per-node", str(gpus), "train_lora.py"]
    args += [str(root / "data" / "pairs.jsonl"), str(root / "run")]
    args += ["--eval-data", str(root / "data" / "pairs.eval.jsonl"), "--base", base]
    _run(args)
    typer.echo(f"adapter and merged model in {root / 'run'}")


@app.command()
def export(merged: Path, out: Path) -> None:
    """Q4_0 GGUF of a merged model, with its size and SHA-256."""
    if not MODEL_NAME.fullmatch(out.name):
        raise typer.BadParameter(
            f"{out.name}: shipped models are named utter-<tier>-v<N>-q4_0.gguf"
        )
    tools = config.data_dir() / "tools" / "llama-cpp"
    _run(
        [
            "python",
            "export_gguf.py",
            str(merged.resolve()),
            str(out.resolve()),
            "--tools",
            str(tools),
        ]
    )


@app.command()
def pin(
    repo: str,
    revision: str,
    path: str = typer.Option("", help="Subfolder in the repo."),
    only: list[str] = typer.Option([], help="Only these file names."),
) -> None:
    """components.json entries (pinned URL, SHA-256, size) for a model repo's files."""
    typer.echo(json.dumps(hub.pin_files(repo, revision, path, only), indent=2))
