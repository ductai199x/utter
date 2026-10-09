"""Download the cleanup LLM candidates (llm_candidates.json) into the Hugging Face cache, and
print each one's local path.

Run: uv run python -m eval.llm_fetch [NAMES...]
"""

from __future__ import annotations

import json
from pathlib import Path

import typer
from huggingface_hub import hf_hub_download

CANDIDATES = Path(__file__).with_name("llm_candidates.json")


def candidates() -> list[dict]:
    return json.loads(CANDIDATES.read_text(encoding="utf-8"))["candidates"]


def local_path(c: dict) -> Path:
    return Path(hf_hub_download(c["repo"], c["file"]))


def main(names: list[str] = typer.Argument(None)) -> None:
    for c in candidates():
        if names and c["name"] not in names:
            continue
        print(f"{c['name']}\t{local_path(c)}", flush=True)


if __name__ == "__main__":
    typer.run(main)
