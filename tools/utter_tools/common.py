"""What every command shares: where things are, the Hub, JSONL files and Utter's own binaries.

The data folder is UTTER_DATA_DIR, else the development folder the Rust core uses for a dev build
(%LOCALAPPDATA%\\Utter_dev, ~/.local/share/utter_dev). Under it: runtime/ (components), eval/
(sets and results), data/ (batches), train/ (runs), models/ (speech exports).
"""

from __future__ import annotations

import json
import os
import sys
from collections.abc import Iterator
from pathlib import Path

import pyarrow.parquet as pq
from huggingface_hub import HfApi, HfFileSystem, snapshot_download

REPO = Path(__file__).resolve().parents[2]
TEXT_REPO = "ductai199x/dictation-to-text"
SPEECH_REPO = "ductai199x/dictation-speech"
PROMPTS = Path(__file__).parent / "prompts"


def data_dir() -> Path:
    if env := os.environ.get("UTTER_DATA_DIR"):
        return Path(env)
    if sys.platform == "win32":
        return (
            Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local") / "Utter_dev"
        )
    return Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local" / "share") / "utter_dev"


def eval_dir() -> Path:
    return data_dir() / "eval"


def prompt(name: str) -> str:
    return (PROMPTS / f"{name}.md").read_text(encoding="utf-8").strip()


def read_jsonl(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    with path.open(encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def write_jsonl(path: Path, rows: list[dict], append: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a" if append else "w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


def component(prefix: str) -> Path:
    """An installed runtime component's folder (utter-cli setup)."""
    dirs = sorted(p for p in (data_dir() / "runtime").glob(f"{prefix}-*") if p.is_dir())
    if not dirs:
        raise FileNotFoundError(f"component {prefix} not installed: utter-cli setup")
    return dirs[-1]


def utter_cli() -> str:
    """UTTER_CLI (a build elsewhere, e.g. Linux), else this repository's release build."""
    exe = "utter-cli.exe" if os.name == "nt" else "utter-cli"
    return os.environ.get("UTTER_CLI") or str(REPO / "target" / "release" / exe)


def llama_server() -> tuple[str, dict[str, str]]:
    """UTTER_LLAMA_SERVER (built elsewhere, system CUDA), else Utter's pinned components."""
    env = dict(os.environ)
    if exe := os.environ.get("UTTER_LLAMA_SERVER"):
        return exe, env
    llama, cuda = component("llama-cpp"), component("cuda-runtime")
    env["PATH"] = os.pathsep.join([str(llama), str(cuda), env.get("PATH", "")])
    return str(llama / ("llama-server.exe" if os.name == "nt" else "llama-server")), env


# --- The Hub, at a pinned revision (a commit), so a run can be repeated.


def revision(repo: str, ref: str | None = None) -> str:
    sha = HfApi().dataset_info(repo, revision=ref).sha
    assert sha, f"{repo}: no commit for {ref}"
    return sha


def hub_files(repo: str, rev: str, pattern: str) -> list[Path]:
    root = Path(
        snapshot_download(repo, repo_type="dataset", revision=rev, allow_patterns=[pattern])
    )
    return sorted(root.glob(pattern))


def text_rows(rev: str, split: str, columns: list[str] | None = None) -> list[dict]:
    files = hub_files(TEXT_REPO, rev, f"data/{split}-*.parquet")
    return [r for f in files for r in pq.read_table(f, columns=columns).to_pylist()]


def speech_rows(rev: str, collection: str, split: str) -> Iterator[dict]:
    """One collection's rows, a row group at a time (audio is large)."""
    for f in hub_files(SPEECH_REPO, rev, f"data/{collection}/{split}-*.parquet"):
        pf = pq.ParquetFile(f)
        for i in range(pf.num_row_groups):
            yield from pf.read_row_group(i).to_pylist()


def speech_columns(rev: str, columns: list[str]) -> list[dict]:
    """Some columns of every speech row (not the audio), read remotely, each with its split."""
    fs = HfFileSystem()
    out = []
    for path in sorted(fs.glob(f"datasets/{SPEECH_REPO}@{rev}/data/*/*.parquet")):
        split = path.rsplit("/", 1)[-1].split("-")[0]
        for r in pq.read_table(path, columns=columns, filesystem=fs).to_pylist():
            out.append({**r, "split": split})
    return out


def speech_collections(rev: str) -> list[str]:
    files = HfApi().list_repo_files(SPEECH_REPO, repo_type="dataset", revision=rev)
    return sorted({f.split("/")[1] for f in files if f.startswith("data/") and f.count("/") == 2})
