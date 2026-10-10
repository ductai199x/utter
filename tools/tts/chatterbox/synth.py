"""Speaks renditions with Chatterbox (MIT), cloning voices from Kyutai's donated voices (CC0,
kyutai/tts-voices, voice-donations/*_enhanced.wav), into an eval set Utter's benches can read.

Voices are split by a hash of their id: eval renditions use only eval voices and train renditions
only train voices, so a fine-tuned model is never scored on a voice it trained on. Each
rendition gets a voice and an expressiveness (exaggeration 0.35-0.75) chosen by a hash of its id.

Output and manifest format match tools/tts/kokoro/synth.py.

Run (from tools/tts/chatterbox):
  UV_PROJECT_ENVIRONMENT=C:/Users/Tai/.cache/utter/envs/tts-chatterbox uv run python synth.py \
      RENDITIONS PASSAGES --name chatterbox --split eval
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

VOICE_REPO = "kyutai/tts-voices"
VOICE_REVISION = "323332d33f997de8394f24a193e1a76df720e01a"
RATE = 16_000


def eval_dir() -> Path:
    env = os.environ.get("UTTER_DATA_DIR")
    return (Path(env) if env else Path.home() / ".cache" / "utter") / "eval"


def _unit(key: str) -> float:
    return int(hashlib.sha256(key.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF


def voices(split: str) -> list[str]:
    """Donated voices (their enhanced recordings) assigned to a split: 30% eval, 70% train."""
    from huggingface_hub import HfApi

    files = HfApi().list_repo_files(VOICE_REPO, revision=VOICE_REVISION)
    ids = sorted(f for f in files if f.startswith("voice-donations/") and f.endswith("_enhanced.wav"))
    return [f for f in ids if (_unit(f + ":split") < 0.3) == (split == "eval")]


def main(
    renditions: Path,
    passages: Path,
    name: str = "chatterbox",
    split: str = "eval",
    limit: int = 0,
    shard: str = typer.Option("0/1", help="I/N: speak only this share, for N processes at once."),
) -> None:
    """Resumes where a previous run stopped. With N > 1, shard I writes manifest.I-of-N.jsonl;
    concatenate the parts into manifest.jsonl when all are done."""
    import torch
    from chatterbox.tts import ChatterboxTTS
    from huggingface_hub import hf_hub_download

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
    pool = voices(split)
    print(f"{len(pool)} {split} voices", flush=True)

    out = eval_dir() / f"en-finaltext-{name}"
    out.mkdir(parents=True, exist_ok=True)
    manifest = out / ("manifest.jsonl" if count == 1 else f"manifest.{index}-of-{count}.jsonl")
    done = set()
    for part in out.glob("manifest*.jsonl"):
        done |= {json.loads(line)["id"] for line in part.open(encoding="utf-8")}

    model = ChatterboxTTS.from_pretrained(device="cuda")
    with manifest.open("a", encoding="utf-8") as fh:
        for i, r in enumerate(rows):
            rid = f"{r['passage_id']}-{r['kind']}"
            if rid in done:
                continue
            voice = pool[int(_unit(rid + ":voice") * len(pool)) % len(pool)]
            ref = hf_hub_download(VOICE_REPO, voice, revision=VOICE_REVISION)
            exaggeration = 0.35 + 0.4 * _unit(rid + ":exaggeration")
            with torch.inference_mode():
                wav = model.generate(r["spoken"], audio_prompt_path=ref, exaggeration=exaggeration, cfg_weight=0.5)
            audio = soxr.resample(wav.squeeze(0).cpu().numpy().astype(np.float32), model.sr, RATE)
            pad = np.zeros(int(0.25 * RATE), dtype=np.float32)
            audio = np.concatenate([pad, audio, pad])
            wav_name = f"{rid}.wav"
            sf.write(out / wav_name, audio, RATE, subtype="PCM_16")
            p = gold[r["passage_id"]]
            fh.write(
                json.dumps(
                    {
                        "id": rid,
                        "audio": wav_name,
                        "text": r["spoken"],
                        "lang": "en",
                        "duration_s": round(len(audio) / RATE, 2),
                        "source": f"en-finaltext-{name}",
                        "gold": p["text"],
                        "domain": p["domain"],
                        "kind": r["kind"],
                        "tags": r["tags"],
                        "voice": voice,
                        "exaggeration": round(exaggeration, 3),
                        "tts": "chatterbox-0.1.7",
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
            fh.flush()
            if (i + 1) % 50 == 0:
                print(f"{i + 1}/{len(rows)}", flush=True)
    print(f"done -> {out}")


if __name__ == "__main__":
    typer.run(main)
