"""Reading the published datasets at a pinned revision (a commit, so a run can be repeated).

Files are fetched into the Hugging Face cache once, and only the shards a call needs: the speech
dataset is 33 GB, and a test collection is a few hundred MB.
"""

from __future__ import annotations

import hashlib
from collections.abc import Iterator
from pathlib import Path

import pyarrow.parquet as pq
from huggingface_hub import HfApi, HfFileSystem, hf_hub_download, snapshot_download
from huggingface_hub.hf_api import RepoFile


def revision(repo: str, ref: str | None = None) -> str:
    """The commit `ref` (a branch, tag or commit; default: main) is at."""
    sha = HfApi().dataset_info(repo, revision=ref).sha
    if sha is None:
        raise RuntimeError(f"{repo}: no commit for {ref or 'main'}")
    return sha


def _files(repo: str, rev: str, pattern: str) -> list[Path]:
    root = Path(
        snapshot_download(repo, repo_type="dataset", revision=rev, allow_patterns=[pattern])
    )
    return sorted(root.glob(pattern))


def text_rows(repo: str, rev: str, split: str, columns: list[str] | None = None) -> list[dict]:
    """Every row of one split of the text dataset."""
    return [
        row
        for f in _files(repo, rev, f"data/{split}-*.parquet")
        for row in pq.read_table(f, columns=columns).to_pylist()
    ]


def speech_collections(repo: str, rev: str) -> list[str]:
    """The speech dataset's collections (its `data/<collection>/` folders)."""
    files = HfApi().list_repo_files(repo, repo_type="dataset", revision=rev)
    return sorted({f.split("/")[1] for f in files if f.startswith("data/") and f.count("/") == 2})


def speech_rows(
    repo: str, rev: str, collection: str, split: str, columns: list[str] | None = None
) -> Iterator[dict]:
    """One collection's rows in one split, a row group at a time (audio is large)."""
    for f in _files(repo, rev, f"data/{collection}/{split}-*.parquet"):
        pf = pq.ParquetFile(f)
        for i in range(pf.num_row_groups):
            yield from pf.read_row_group(i, columns=columns).to_pylist()


def speech_columns(repo: str, rev: str, columns: list[str]) -> list[dict]:
    """Some columns of every speech row (not the audio), each row with its `split`: read over the
    Hub's file system, so only those columns are transferred."""
    fs = HfFileSystem()
    rows = []
    for path in sorted(fs.glob(f"datasets/{repo}@{rev}/data/*/*.parquet")):
        split = path.rsplit("/", 1)[-1].split("-")[0]
        rows += [
            {**r, "split": split}
            for r in pq.read_table(path, columns=columns, filesystem=fs).to_pylist()
        ]
    return rows


def pin_files(
    repo: str, rev: str, path: str = "", only: list[str] | None = None, repo_type: str = "model"
) -> list[dict]:
    """components.json `files` entries for a repo's files at a commit: each file's pinned URL
    (the commit, never a branch), SHA-256 and size."""
    if len(rev) != 40:
        raise ValueError("pin to a full 40-character commit, not a branch")
    out = []
    tree = HfApi().list_repo_tree(
        repo, revision=rev, path_in_repo=path or None, expand=True, repo_type=repo_type
    )
    for f in tree:
        name = f.path.split("/")[-1]
        if not isinstance(f, RepoFile) or name == "README.md" or name.startswith("."):
            continue
        if only and name not in only:
            continue
        if f.lfs:
            sha, size = f.lfs.sha256, f.lfs.size
        else:
            local = hf_hub_download(repo, f.path, revision=rev, repo_type=repo_type)
            data = Path(local).read_bytes()
            sha, size = hashlib.sha256(data).hexdigest(), len(data)
        out.append(
            {
                "name": name,
                "url": f"https://huggingface.co/{repo}/resolve/{rev}/{f.path}",
                "sha256": sha,
                "size": size,
            }
        )
    return out
