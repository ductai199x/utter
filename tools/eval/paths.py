"""Where Utter's development data lives on this machine: eval sets, results, training runs,
local model exports, and the models the dev builds install.

UTTER_DATA_DIR if set, else the development data folder the Rust core resolves for a dev
build (crates/utter-core, data_dir): %LOCALAPPDATA%\\Utter_dev on Windows,
~/.local/share/utter_dev on Linux, ~/Library/Application Support/Utter_dev on macOS. An
installed Utter uses the sibling folder without "_dev" and never shares these files. Models
under evaluation stay in the standard Hugging Face cache. docs/local-setup.md records the same.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path


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
