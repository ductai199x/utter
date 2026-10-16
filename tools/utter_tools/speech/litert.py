"""The encoder for Android's GPU: runs envs/litert/export_litert.py in its own environment.

LiteRT's converter ships for Linux and macOS only, so on Windows the environment lives in WSL
(the default distribution, with uv installed), in WSL's own file system: building it on the
Windows drive is slow.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from utter_tools import common

ENV = common.REPO / "tools" / "envs" / "litert"
WSL_VENV = "$HOME/.cache/utter/envs/litert"


def _wsl_path(path: Path) -> str:
    out = subprocess.run(
        ["wsl", "--", "wslpath", "-a", path.resolve().as_posix()],
        check=True,
        capture_output=True,
        text=True,
    )
    return out.stdout.strip()


def export_litert(out: Path, frames: str, weights: str) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    args = ["--frames", frames, "--weights", weights]
    if sys.platform == "win32":
        env, target = _wsl_path(ENV), _wsl_path(out)
        command = (
            f'cd "{env}" && UV_PROJECT_ENVIRONMENT="{WSL_VENV}" uv run --locked --project . '
            f'python export_litert.py "{target}" {" ".join(args)}'
        )
        subprocess.run(["wsl", "--", "bash", "-lc", command], check=True)
    else:
        subprocess.run(
            [
                "uv",
                "run",
                "--locked",
                "--project",
                str(ENV),
                "python",
                "export_litert.py",
                str(out.resolve()),
                *args,
            ],
            check=True,
            cwd=ENV,
        )
