"""Turns a fine-tuned, merged model into the Q4_0 GGUF the product loads: convert with llama.cpp's
converter at the runtime's build, then quantize to Q4_0 (the format the QAT checkpoint was
trained for), and record the file's SHA-256 so it can be pinned as a component.

Run: UV_PROJECT_ENVIRONMENT=%USERPROFILE%/.cache/utter/envs/train uv run python export_gguf.py MERGED OUT.gguf
"""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

import typer
from llama_tools import data_dir, install


def main(merged: Path, out: Path) -> None:
    tools = install()
    # llama-quantize cannot open paths past Windows' 260-character limit, so both files are
    # made in a short scratch folder and the result is moved into place.
    work = data_dir() / "tmp" / "export"
    work.mkdir(parents=True, exist_ok=True)
    f16 = work / "model.f16.gguf"
    q4 = work / "model.q4_0.gguf"
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join([str(tools / "gguf-py"), env.get("PYTHONPATH", "")])
    subprocess.run(
        [
            sys.executable,
            str(tools / "convert_hf_to_gguf.py"),
            str(merged),
            "--outfile",
            str(f16),
            "--outtype",
            "f16",
        ],
        check=True,
        env=env,
    )
    subprocess.run([str(tools / "llama-quantize.exe"), str(f16), str(q4), "Q4_0"], check=True)
    f16.unlink()
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(q4, out)
    h = hashlib.sha256()
    with out.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    print(f"{out}: {out.stat().st_size} bytes, sha256 {h.hexdigest()}")


if __name__ == "__main__":
    typer.run(main)
