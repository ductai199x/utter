"""Turns a fine-tuned, merged model into the Q4_0 GGUF the app loads: convert with llama.cpp's
converter at the exact build Utter runs (the `llama-cpp` runtime component), quantize to Q4_0 (the
format the QAT checkpoint was trained for), and print the file's size and SHA-256 for its
components.json entry.

The tools come from one of:
- UTTER_LLAMA_BUILD: a llama.cpp source tree at that build, built (build/bin/llama-quantize),
  e.g. on Linux, where the component has no release to download;
- otherwise the component's own release zip (its URL and SHA-256 from components.json, so it is
  checksummed) for llama-quantize, and llama.cpp's source at the same tag for the converter,
  both unpacked once into `--tools`.

Runs in the training environment (the converter needs torch and safetensors); utter-train export
starts it:  python export_gguf.py MERGED OUT.gguf --tools DIR
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]


def component() -> dict:
    comps = json.loads(
        (REPO / "crates" / "utter-runtime" / "components.json").read_text(encoding="utf-8")
    )
    items = comps["components"] if isinstance(comps, dict) else comps
    return next(c for c in items if c["id"] == "llama-cpp")


def _fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=600) as r:
        return r.read()


def install(dest: Path) -> tuple[Path, Path]:
    """The converter's folder (convert_hf_to_gguf.py, gguf-py, conversion) and llama-quantize."""
    if build := os.environ.get("UTTER_LLAMA_BUILD"):
        tree = Path(build)
        return tree, tree / "build" / "bin" / "llama-quantize"
    quantize = dest / "llama-quantize.exe"
    if quantize.is_file() and (dest / "convert_hf_to_gguf.py").is_file():
        return dest, quantize
    if os.name != "nt":
        raise RuntimeError(
            "no llama.cpp release for this platform: set UTTER_LLAMA_BUILD to a built tree"
        )
    c = component()
    dest.mkdir(parents=True, exist_ok=True)
    blob = _fetch(c["url"])
    digest = hashlib.sha256(blob).hexdigest()
    if digest != c["sha256"]:
        raise RuntimeError(f"{c['url']}: sha256 {digest}, expected {c['sha256']}")
    with zipfile.ZipFile(io.BytesIO(blob)) as z:
        for name in z.namelist():
            base = name.rsplit("/", 1)[-1]
            if base == "llama-quantize.exe" or base.endswith(".dll"):
                (dest / base).write_bytes(z.read(name))
    src = _fetch(f"https://github.com/ggml-org/llama.cpp/archive/refs/tags/{c['version']}.tar.gz")
    print(f"llama.cpp {c['version']} source: sha256 {hashlib.sha256(src).hexdigest()}")
    with tarfile.open(fileobj=io.BytesIO(src), mode="r:gz") as t:
        for m in t.getmembers():
            parts = m.name.split("/", 1)
            if len(parts) < 2 or not m.isfile():
                continue
            rel = parts[1]
            # The converter imports gguf-py and the conversion package beside it.
            if rel == "convert_hf_to_gguf.py" or rel.startswith(("gguf-py/", "conversion/")):
                target = dest / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                f = t.extractfile(m)
                assert f is not None
                target.write_bytes(f.read())
    return dest, quantize


def main() -> None:
    ap = argparse.ArgumentParser(description="Merged model -> Q4_0 GGUF")
    ap.add_argument("merged", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument(
        "--tools", type=Path, required=True, help="Where the llama.cpp tools are unpacked"
    )
    args = ap.parse_args()
    converter, quantize = install(args.tools)
    # llama-quantize cannot open paths past Windows' 260-character limit, so both files are made in
    # a short scratch folder and the result is moved into place.
    work = args.tools / "work"
    work.mkdir(parents=True, exist_ok=True)
    f16, q4 = work / "model.f16.gguf", work / "model.q4_0.gguf"
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join([str(converter / "gguf-py"), env.get("PYTHONPATH", "")])
    convert = [sys.executable, str(converter / "convert_hf_to_gguf.py"), str(args.merged)]
    subprocess.run([*convert, "--outfile", str(f16), "--outtype", "f16"], check=True, env=env)
    subprocess.run([str(quantize), str(f16), str(q4), "Q4_0"], check=True)
    f16.unlink()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(q4, args.out)
    h = hashlib.sha256()
    with args.out.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    print(
        json.dumps(
            {"file": args.out.name, "size": args.out.stat().st_size, "sha256": h.hexdigest()}
        )
    )


if __name__ == "__main__":
    main()
