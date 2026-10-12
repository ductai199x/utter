"""Where things are: the data folder, the Hub repos, the repository, and Utter's own binaries.

The data folder is UTTER_DATA_DIR if set, else the development data folder the Rust core resolves
for a dev build (crates/utter-core, data_dir): %LOCALAPPDATA%\\Utter_dev on Windows,
~/.local/share/utter_dev on Linux, ~/Library/Application Support/Utter_dev on macOS. An installed
Utter uses the sibling folder without "_dev" and never shares these files.

Layout under it (docs/local-setup.md):
  runtime/         components utter-cli setup installs (llama.cpp, CUDA, models)
  eval/<set>/      an eval set as utter-cli bench reads it (manifest.jsonl and audio)
  eval/results/    asr/<run>/, llm/<run>/, score/, judge/, reports
  data/            the data generator's working files (utter-data)
  train/<run>/     training runs (utter-train)
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

TEXT_REPO = "ductai199x/dictation-to-text"
SPEECH_REPO = "ductai199x/dictation-speech"


def data_dir() -> Path:
    env = os.environ.get("UTTER_DATA_DIR")
    if env:
        return Path(env)
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
        return base / "Utter_dev"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "Utter_dev"
    xdg = os.environ.get("XDG_DATA_HOME")
    return (Path(xdg) if xdg else Path.home() / ".local" / "share") / "utter_dev"


def eval_dir() -> Path:
    return data_dir() / "eval"


def results_dir() -> Path:
    return eval_dir() / "results"


def work_dir() -> Path:
    """The data generator's working files."""
    return data_dir() / "data"


def runtime_dir() -> Path:
    return data_dir() / "runtime"


def component_dir(prefix: str) -> Path:
    """An installed runtime component's folder (the newest version), e.g. `llama-cpp`."""
    dirs = sorted(
        p for p in runtime_dir().glob(f"{prefix}-*") if p.is_dir() and not p.name.endswith(".part")
    )
    if not dirs:
        raise FileNotFoundError(f"runtime component {prefix} is not installed (utter-cli setup)")
    return dirs[-1]


def utter_cli() -> Path:
    """utter-cli: UTTER_CLI if set (a build elsewhere, e.g. Linux), else the release build here."""
    env = os.environ.get("UTTER_CLI")
    if env:
        return Path(env)
    exe = "utter-cli.exe" if os.name == "nt" else "utter-cli"
    for target in (os.environ.get("CARGO_TARGET_DIR"), REPO / "target"):
        if target and (Path(target) / "release" / exe).is_file():
            return Path(target) / "release" / exe
    raise FileNotFoundError("utter-cli not built: cargo build --release -p utter-cli")


def llama_server() -> tuple[Path, dict[str, str]]:
    """llama-server and the environment it runs in: UTTER_LLAMA_SERVER if set (a build elsewhere,
    with the system's CUDA), else Utter's own pinned components."""
    env = dict(os.environ)
    if exe := os.environ.get("UTTER_LLAMA_SERVER"):
        return Path(exe), env
    llama, cuda = component_dir("llama-cpp"), component_dir("cuda-runtime")
    env["PATH"] = os.pathsep.join([str(llama), str(cuda), env.get("PATH", "")])
    return llama / ("llama-server.exe" if os.name == "nt" else "llama-server"), env
