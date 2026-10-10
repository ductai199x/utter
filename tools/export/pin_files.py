"""Print the `files` entries for components.json: each file's pinned URL (commit, not branch),
SHA-256 and size, from the Hugging Face Hub.

Run: uv run python -m export.pin_files REPO REVISION [--path SUBFOLDER] [--only NAME ...]
"""

from __future__ import annotations

import hashlib
import json

import typer
from huggingface_hub import HfApi, hf_hub_download


def main(
    repo: str,
    revision: str,
    path: str = typer.Option("", help="Subfolder in the repo."),
    only: list[str] = typer.Option([], help="Only these file names."),
) -> None:
    if len(revision) != 40:
        raise typer.BadParameter("pin to a full 40-character commit, not a branch")
    api = HfApi()
    out = []
    for f in api.list_repo_tree(repo, revision=revision, path_in_repo=path or None, expand=True):
        name = f.path.split("/")[-1]
        if name == "README.md" or name.startswith(".") or not hasattr(f, "size"):
            continue
        if only and name not in only:
            continue
        if f.lfs:
            sha, size = f.lfs.sha256, f.lfs.size
        else:
            local = hf_hub_download(repo, f.path, revision=revision)
            with open(local, "rb") as fh:
                data = fh.read()
            sha, size = hashlib.sha256(data).hexdigest(), len(data)
        url = f"https://huggingface.co/{repo}/resolve/{revision}/{f.path}"
        out.append({"name": name, "url": url, "sha256": sha, "size": size})
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    typer.run(main)
