"""Speaks again the rows of a Kokoro set whose audio is missing, from its manifest: the same text,
voice and speed the row records, the same processing as synth.py (16 kHz, 0.25 s of silence
around). Kokoro is deterministic, so a row's length matches its recorded duration_s; a row that
does not is reported.

Run (from tools/tts/kokoro): uv run python respeak.py en-finaltext-train-kokoro-v2 --shard 0/4
"""

from __future__ import annotations

import json

import numpy as np
import soundfile as sf
import soxr
import typer
from synth import KOKORO_RATE, RATE, eval_dir


def main(
    name: str,
    shard: str = typer.Option(
        "0/1", help="I/N: only rows I, I+N, I+2N... for N processes at once."
    ),
) -> None:
    from kokoro import KPipeline

    index, count = (int(x) for x in shard.split("/"))
    out = eval_dir() / name
    rows = [json.loads(line) for line in (out / "manifest.jsonl").open(encoding="utf-8")]
    rows = [
        r for i, r in enumerate(rows) if i % count == index and not (out / r["audio"]).is_file()
    ]
    pipelines: dict[str, KPipeline] = {}
    mismatched = 0
    for i, r in enumerate(rows):
        lang = r["voice"][0]
        if lang not in pipelines:
            pipelines[lang] = KPipeline(lang_code=lang, repo_id="hexgrad/Kokoro-82M")
        chunks = [a for _, _, a in pipelines[lang](r["text"], voice=r["voice"], speed=r["speed"])]
        if not chunks:
            print(f"{r['id']}: no audio", flush=True)
            continue
        audio = np.concatenate(
            [c.numpy() if hasattr(c, "numpy") else np.asarray(c) for c in chunks]
        )
        audio = soxr.resample(audio.astype(np.float32), KOKORO_RATE, RATE)
        pad = np.zeros(int(0.25 * RATE), dtype=np.float32)
        audio = np.concatenate([pad, audio, pad])
        duration = round(len(audio) / RATE, 2)
        if abs(duration - r["duration_s"]) > 0.05:
            mismatched += 1
            print(f"{r['id']}: {duration} s, the manifest says {r['duration_s']} s", flush=True)
        # Written whole, then renamed: an interrupted run never leaves a cut-short file.
        tmp = out / (r["audio"] + ".part")
        sf.write(tmp, audio, RATE, subtype="PCM_16", format="WAV")
        tmp.replace(out / r["audio"])
        if (i + 1) % 500 == 0:
            print(f"{i + 1}/{len(rows)}", flush=True)
    print(f"done: {len(rows)} rows, {mismatched} with another length -> {out}")


if __name__ == "__main__":
    typer.run(main)
