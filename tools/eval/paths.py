"""Where Utter's eval data lives on this machine.

Models stay in the standard Hugging Face cache. Eval clips go under the data folder:
UTTER_DATA_DIR if set, else ~/.cache/utter (2026-10-09: on C:). docs/local-setup.md
records the same.
"""

from __future__ import annotations

import os
from pathlib import Path


def data_dir() -> Path:
    env = os.environ.get("UTTER_DATA_DIR")
    return Path(env) if env else Path.home() / ".cache" / "utter"


def eval_dir() -> Path:
    return data_dir() / "eval"
