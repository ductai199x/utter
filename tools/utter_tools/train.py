"""utter-train: fine-tune a cleanup model on dictation-to-text and export the GGUF the app loads.

Training reads the dataset at one revision (recorded beside the run) and uses the recipe Quality
and Light are trained with (envs/train/train_lora.py's defaults); the export is Q4_0 with
llama.cpp at the runtime's build, and prints the file's size and SHA-256 for components.json. The
heavy work runs in its own environment (envs/train), started from here.

    utter-train --name quality-v3 --base e4b --gpus 2 --gguf utter-quality-v3-q4_0.gguf
    utter-train --merged RUN/merged --gguf utter-quality-v3-q4_0.gguf      (export only)
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path

import typer

from utter_tools import common

ENV = common.REPO / "tools" / "envs" / "train"
# Shipped models are named utter-<tier>-v<N>-<quantization>.gguf.
MODEL_NAME = re.compile(r"utter-(quality|light)-v\d+-q4_0\.gguf")


def _in_env(args: list[str]) -> None:
    subprocess.run(["uv", "run", "--project", str(ENV), *args], check=True, cwd=ENV)


def write_data(folder: Path, ref: str | None) -> dict:
    """The dataset's splits as the trainer's JSONL ({"prompt": [system, user], "completion":
    [assistant]}), and where they came from (source.json)."""
    rev = common.revision(common.TEXT_REPO, ref)
    record: dict = {"dataset": common.TEXT_REPO, "revision": rev, "files": {}}
    for split, name in (("train", "pairs.jsonl"), ("test", "pairs.eval.jsonl")):
        rows = common.text_rows(rev, split, ["id", "messages"])
        pairs = [
            {"id": r["id"], "prompt": r["messages"][:2], "completion": r["messages"][2:]}
            for r in rows
        ]
        common.write_jsonl(folder / name, pairs)
        record["files"][name] = len(pairs)
    (folder / "source.json").write_text(json.dumps(record, indent=2))
    return record


def main(
    name: str = typer.Option("", help="The run's name, in <data dir>/train/<name>/."),
    base: str = typer.Option("e4b", help="e4b (Quality) or e2b (Light)."),
    gpus: int = typer.Option(1, help="GPUs on this machine."),
    ref: str = typer.Option(None, help="dictation-to-text revision (default: main)."),
    gguf: Path = typer.Option(None, help="Export to this file (utter-<tier>-v<N>-q4_0.gguf)."),
    merged: Path = typer.Option(None, help="Export this merged model instead of training."),
) -> None:
    """Fine-tune a cleanup model on the text dataset with the shipped recipe; export its GGUF."""
    if gguf and not MODEL_NAME.fullmatch(gguf.name):
        raise typer.BadParameter(
            f"{gguf.name}: shipped models are named utter-<tier>-v<N>-q4_0.gguf"
        )
    if merged is None:
        if not name:
            raise typer.BadParameter("give --name to train, or --merged to export")
        run = common.data_dir() / "train" / name
        if (run / "run").exists():
            raise typer.BadParameter(f"{run / 'run'} exists: choose another name")
        record = write_data(run / "data", ref)
        print(f"{common.TEXT_REPO} @ {record['revision'][:10]}: {record['files']}")
        args = ["torchrun", "--nproc-per-node", str(gpus), "train_lora.py"]
        args += [str(run / "data" / "pairs.jsonl"), str(run / "run")]
        args += ["--eval-data", str(run / "data" / "pairs.eval.jsonl"), "--base", base]
        _in_env(args)
        merged = run / "run" / "merged"
    if gguf:
        tools = common.data_dir() / "tools" / "llama-cpp"
        _in_env(
            [
                "python",
                "export_gguf.py",
                str(merged.resolve()),
                str(gguf.resolve()),
                "--tools",
                str(tools),
            ]
        )


def cli() -> None:
    typer.run(main)
