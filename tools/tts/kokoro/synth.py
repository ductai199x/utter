"""Speaks renditions with Kokoro-82M into an eval set Utter's benches can read.

Input: renditions JSONL (tools/eval/finaltext/spoken.py). Output: <eval dir>/en-finaltext-<name>/
with one 16 kHz mono WAV per rendition and a manifest.jsonl in the eval-set format, whose `text`
is what was spoken (so WER measures the recognizer alone); the passage, the kind and the tags
ride along for the final-text scorer.

Voices: twelve of Kokoro's better-graded English voices (American and British, women and men),
chosen per rendition by a hash of its id, at a speaking rate between 0.9 and 1.1.

Run (from tools/tts/kokoro): uv run python synth.py RENDITIONS PASSAGES --name kokoro
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

import numpy as np
import soundfile as sf
import soxr
import typer

VOICES = [
    ("a", "af_heart"),
    ("a", "af_bella"),
    ("a", "af_nicole"),
    ("a", "af_aoede"),
    ("a", "af_kore"),
    ("a", "af_sarah"),
    ("a", "am_fenrir"),
    ("a", "am_michael"),
    ("a", "am_puck"),
    ("b", "bf_emma"),
    ("b", "bm_george"),
    ("b", "bm_fable"),
]
KOKORO_RATE = 24_000
RATE = 16_000


def eval_dir() -> Path:
    env = os.environ.get("UTTER_DATA_DIR")
    return (Path(env) if env else Path.home() / ".cache" / "utter") / "eval"


def _unit(key: str) -> float:
    return int(hashlib.sha256(key.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF


def main(
    renditions: Path,
    passages: Path,
    name: str = "kokoro",
    limit: int = 0,
    shard: str = typer.Option("0/1", help="I/N: speak only this share, for N processes at once."),
) -> None:
    """Resumes where a previous run stopped. With N > 1, shard I writes manifest.I-of-N.jsonl;
    concatenate the parts into manifest.jsonl when all are done."""
    from kokoro import KPipeline

    index, count = (int(x) for x in shard.split("/"))
    gold = {}
    for line in passages.open(encoding="utf-8"):
        p = json.loads(line)
        gold[p["id"]] = p
    rows = [json.loads(line) for line in renditions.open(encoding="utf-8")]
    # Renditions of passages since dropped from the passage file (sources.py filters) are not spoken.
    rows = [r for r in rows if r["passage_id"] in gold]
    rows = [r for r in rows if int(_unit(f"{r['passage_id']}-{r['kind']}:shard") * count) % count == index]
    if limit:
        rows = rows[:limit]

    out = eval_dir() / f"en-finaltext-{name}"
    out.mkdir(parents=True, exist_ok=True)
    manifest = out / ("manifest.jsonl" if count == 1 else f"manifest.{index}-of-{count}.jsonl")
    done = set()
    for part in out.glob("manifest*.jsonl"):
        done |= {json.loads(line)["id"] for line in part.open(encoding="utf-8")}

    pipelines = {lang: KPipeline(lang_code=lang, repo_id="hexgrad/Kokoro-82M") for lang in {v[0] for v in VOICES}}
    with manifest.open("a", encoding="utf-8") as fh:
        for i, r in enumerate(rows):
            rid = f"{r['passage_id']}-{r['kind']}"
            if rid in done:
                continue
            lang, voice = VOICES[int(_unit(rid + ":voice") * len(VOICES)) % len(VOICES)]
            speed = 0.9 + 0.2 * _unit(rid + ":speed")
            chunks = [audio for _, _, audio in pipelines[lang](r["spoken"], voice=voice, speed=speed)]
            if not chunks:
                print(f"{rid}: no audio", flush=True)
                continue
            audio = np.concatenate([c.numpy() if hasattr(c, "numpy") else np.asarray(c) for c in chunks])
            audio = soxr.resample(audio.astype(np.float32), KOKORO_RATE, RATE)
            # A short lead-in and tail of silence, as a real dictation has around the speech.
            pad = np.zeros(int(0.25 * RATE), dtype=np.float32)
            audio = np.concatenate([pad, audio, pad])
            wav = f"{rid}.wav"
            sf.write(out / wav, audio, RATE, subtype="PCM_16")
            p = gold[r["passage_id"]]
            fh.write(
                json.dumps(
                    {
                        "id": rid,
                        "audio": wav,
                        "text": r["spoken"],
                        "lang": "en",
                        "duration_s": round(len(audio) / RATE, 2),
                        "source": f"en-finaltext-{name}",
                        "gold": p["text"],
                        "domain": p["domain"],
                        "kind": r["kind"],
                        "tags": r["tags"],
                        "voice": voice,
                        "speed": round(speed, 3),
                        "tts": "kokoro-82m",
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
            fh.flush()
            if (i + 1) % 100 == 0:
                print(f"{i + 1}/{len(rows)}", flush=True)
    print(f"done -> {out}")


if __name__ == "__main__":
    typer.run(main)
