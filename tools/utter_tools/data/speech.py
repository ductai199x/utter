"""A batch's speech: its renditions spoken by a TTS engine (envs/tts/speak.py, in the engine's own
environment), then transcribed by Utter's recognizer through the Rust runtime (utter-cli bench),
as every eval set is.
"""

from __future__ import annotations

import os
import subprocess

from utter_tools import config
from utter_tools.data.batch import Batch, read_jsonl, write_jsonl
from utter_tools.eval import runs
from utter_tools.eval.sets import asr_path

TOOLS = config.REPO / "tools"
SPEAK = TOOLS / "envs" / "tts" / "speak.py"
# The recognizer run every batch is transcribed with.
ASR = "parakeet"


def speak(batch: Batch, engine: str, voices: str, gpus: list[str], per_gpu: int = 1) -> int:
    """Speaks the batch's renditions, `per_gpu` processes on each of `gpus` at once, then merges
    their manifests; returns the number of clips."""
    out = batch.clips
    count = len(gpus) * per_gpu
    procs = []
    for i in range(count):
        env = {**os.environ, "CUDA_VISIBLE_DEVICES": gpus[i % len(gpus)]}
        cmd = [
            "uv",
            "run",
            "--project",
            str(TOOLS / "envs" / f"tts-{engine}"),
            "python",
            str(SPEAK),
        ]
        cmd += [engine, str(batch.renditions_path), str(out), "--collection", batch.meta.collection]
        cmd += ["--voices", voices, "--shard", f"{i}/{count}"]
        procs.append(subprocess.Popen(cmd, env=env))
    failed = sum(p.wait() != 0 for p in procs)
    if failed:
        raise RuntimeError(f"{failed} of {count} speaking processes failed")
    return merge(batch)


def merge(batch: Batch) -> int:
    """One manifest of the clips whose WAV exists: the shards' rows, the latest per id."""
    out = batch.clips
    rows: dict[str, dict] = {}
    for part in sorted(out.glob("manifest*.jsonl")):
        for r in read_jsonl(part):
            if (out / r["audio"]).is_file():
                rows[r["id"]] = r
    write_jsonl(out / "manifest.jsonl", sorted(rows.values(), key=lambda r: r["id"]))
    for part in out.glob("manifest.*-of-*.jsonl"):
        part.unlink()
    return len(rows)


def transcribe(batch: Batch) -> dict[str, str]:
    """Parakeet on the batch's clips; returns speech row id -> what it heard."""
    runs.asr(batch.meta.collection, ASR)
    return {r["id"]: r["hyp"] for r in read_jsonl(asr_path(ASR, batch.meta.collection))}
