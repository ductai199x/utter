"""llama.cpp's conversion tools at the exact build Utter runs (the `llama-cpp` runtime component),
for turning a fine-tuned model into the GGUF the product loads.

- llama-quantize.exe comes from the same release zip as the component (URL and SHA-256 from
  crates/utter-runtime/components.json), so it is checksummed;
- convert_hf_to_gguf.py and the gguf-py and conversion packages it imports come from
  llama.cpp's source at the same tag.

Both land in <data dir>/tools/llama-<version>/. Dev only.

Run: uv run python llama_tools.py
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import tarfile
import urllib.request
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def data_dir() -> Path:
    env = os.environ.get("UTTER_DATA_DIR")
    return Path(env) if env else Path.home() / ".cache" / "utter"


def component() -> dict:
    comps = json.loads(
        (REPO / "crates" / "utter-runtime" / "components.json").read_text(encoding="utf-8")
    )
    items = comps["components"] if isinstance(comps, dict) else comps
    return next(c for c in items if c["id"] == "llama-cpp")


def tools_dir() -> Path:
    return data_dir() / "tools" / f"llama-{component()['version']}"


def _fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=600) as r:
        return r.read()


def install() -> Path:
    c = component()
    out = tools_dir()
    if all((out / f).exists() for f in ("llama-quantize.exe", "convert_hf_to_gguf.py", "conversion")):
        return out
    out.mkdir(parents=True, exist_ok=True)

    blob = _fetch(c["url"])
    digest = hashlib.sha256(blob).hexdigest()
    if digest != c["sha256"]:
        raise RuntimeError(f"{c['url']}: sha256 {digest}, expected {c['sha256']}")
    with zipfile.ZipFile(io.BytesIO(blob)) as z:
        for name in z.namelist():
            base = name.rsplit("/", 1)[-1]
            if base == "llama-quantize.exe" or base.endswith(".dll"):
                (out / base).write_bytes(z.read(name))

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
                target = out / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                f = t.extractfile(m)
                assert f is not None
                target.write_bytes(f.read())
    return out


if __name__ == "__main__":
    print(install())
