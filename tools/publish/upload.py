"""Uploads a built dataset folder (or part of one) to a private dataset repo on the Hub, creating
the repo if needed. Re-running the same call resumes an interrupted upload.

Run (from tools): uv run python -m publish.upload ductai199x/dictation-to-text OUT_DIR
                  uv run python -m publish.upload ductai199x/dictation-speech OUT_DIR --only "data/train-*/*"
"""

from __future__ import annotations

from pathlib import Path

import typer
from huggingface_hub import HfApi


def main(
    repo: str,
    folder: Path,
    only: list[str] = typer.Option(None, help="Upload only paths matching these patterns"),
    message: str = "Upload",
) -> None:
    api = HfApi()
    api.create_repo(repo, repo_type="dataset", private=True, exist_ok=True)
    patterns = only or ["README.md", "data/**"]
    api.upload_folder(
        repo_id=repo,
        repo_type="dataset",
        folder_path=folder,
        allow_patterns=patterns,
        commit_message=message,
    )
    info = api.dataset_info(repo, files_metadata=True)
    files = [s for s in info.siblings if s.rfilename.startswith("data/") or s.rfilename == "README.md"]
    print(f"{repo}: private={info.private}, {len(files)} files, {sum(s.size or 0 for s in files) / 1e9:.2f} GB")


if __name__ == "__main__":
    typer.run(main)
