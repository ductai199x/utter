"""Speaks a batch's renditions into a speech collection: one 16 kHz mono WAV per rendition and a
manifest of speech rows (dictation-speech's schema, the audio as the WAV's name), the layout
`utter-cli bench` reads.

Runs inside the engine's own environment (engines pin their own torch), so it imports nothing
from utter_tools; utter-data speak starts it:

  uv run --project envs/tts-<engine> python envs/tts/speak.py ENGINE RENDITIONS OUT --collection C

Engines:
- kokoro: Kokoro-82M (Apache-2.0), twelve of its better-graded English voices (American and
  British, women and men) at a speaking rate of 0.9-1.1;
- chatterbox: Chatterbox (MIT), cloning Kyutai's donated voices (CC0, kyutai/tts-voices), with an
  expressiveness (exaggeration) of 0.35-0.75. Donated voices are split 30/70 by a hash of their
  file: `--voices eval` speaks with the first share only, so a model is never scored on a voice it
  trained on. Donors' names stay out of the data: a voice is "donated-voice-NN".

Each rendition's voice and setting come from a hash of its id, so a rerun speaks it the same way.
With `--shard I/N`, N processes share the work, each writing manifest.I-of-N.jsonl (utter-data
merges them). A rerun resumes: a rendition is spoken again only if its WAV is missing.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections.abc import Callable
from pathlib import Path

import numpy as np
import soundfile as sf
import soxr

RATE = 16_000
# Silence before and after the speech, as a real dictation has.
PAD_S = 0.25

KOKORO_RATE = 24_000
KOKORO_VOICES = [
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
VOICE_REPO = "kyutai/tts-voices"
VOICE_REVISION = "323332d33f997de8394f24a193e1a76df720e01a"
EVAL_VOICE_SHARE = 0.3

# (transcript, rendition id) -> (16 kHz audio, engine, voice, settings)
Speaker = Callable[[str, str], tuple[np.ndarray, str, str, list[str]]]


def unit(key: str) -> float:
    return int(hashlib.sha256(key.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF


def kokoro() -> Speaker:
    from kokoro import KPipeline

    pipelines = {lang: KPipeline(lang_code=lang, repo_id="hexgrad/Kokoro-82M") for lang in "ab"}

    def speak(text: str, rid: str) -> tuple[np.ndarray, str, str, list[str]]:
        lang, voice = KOKORO_VOICES[
            int(unit(rid + ":voice") * len(KOKORO_VOICES)) % len(KOKORO_VOICES)
        ]
        speed = 0.9 + 0.2 * unit(rid + ":speed")
        chunks = [audio for _, _, audio in pipelines[lang](text, voice=voice, speed=speed)]
        if not chunks:
            return np.zeros(0, np.float32), "", "", []
        audio = np.concatenate(
            [c.numpy() if hasattr(c, "numpy") else np.asarray(c) for c in chunks]
        )
        audio = soxr.resample(audio.astype(np.float32), KOKORO_RATE, RATE)
        return audio, "kokoro-82m", voice, [f"speed={round(speed, 3)}"]

    return speak


def chatterbox(voices: str) -> Speaker:
    import torch
    from chatterbox.tts import ChatterboxTTS
    from huggingface_hub import HfApi, hf_hub_download

    # Eval voices are numbered first, as the published eval sets name them, then the rest.
    files = HfApi().list_repo_files(VOICE_REPO, revision=VOICE_REVISION)
    donated = sorted(
        f for f in files if f.startswith("voice-donations/") and f.endswith("_enhanced.wav")
    )
    eval_share = [f for f in donated if unit(f + ":split") < EVAL_VOICE_SHARE]
    rest = [f for f in donated if f not in eval_share]
    names = {f: f"donated-voice-{i + 1:02d}" for i, f in enumerate(eval_share + rest)}
    pool = eval_share if voices == "eval" else rest
    model = ChatterboxTTS.from_pretrained(device="cuda")

    def speak(text: str, rid: str) -> tuple[np.ndarray, str, str, list[str]]:
        voice = pool[int(unit(rid + ":voice") * len(pool)) % len(pool)]
        ref = hf_hub_download(VOICE_REPO, voice, revision=VOICE_REVISION)
        exaggeration = 0.35 + 0.4 * unit(rid + ":exaggeration")
        with torch.inference_mode():
            wav = model.generate(
                text, audio_prompt_path=ref, exaggeration=exaggeration, cfg_weight=0.5
            )
        audio = soxr.resample(wav.squeeze(0).cpu().numpy().astype(np.float32), model.sr, RATE)
        return audio, "chatterbox-0.1.7", names[voice], [f"exaggeration={round(exaggeration, 3)}"]

    return speak


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("engine", choices=["kokoro", "chatterbox"])
    ap.add_argument("renditions", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--collection", required=True)
    ap.add_argument("--voices", choices=["eval", "train"], default="train")
    ap.add_argument("--shard", default="0/1", help="I/N: speak only this share")
    args = ap.parse_args()

    index, count = (int(x) for x in args.shard.split("/"))
    rows = [json.loads(line) for line in args.renditions.open(encoding="utf-8") if line.strip()]
    rows = [r for r in rows if int(unit(r["id"] + ":shard") * count) % count == index]
    out: Path = args.out
    out.mkdir(parents=True, exist_ok=True)
    todo = [r for r in rows if not (out / f"{r['id']}.wav").is_file()]
    print(f"{len(rows)} renditions in this shard, {len(todo)} to speak", flush=True)
    if not todo:
        return
    speak = kokoro() if args.engine == "kokoro" else chatterbox(args.voices)
    manifest = out / ("manifest.jsonl" if count == 1 else f"manifest.{index}-of-{count}.jsonl")
    pad = np.zeros(int(PAD_S * RATE), dtype=np.float32)
    with manifest.open("a", encoding="utf-8", newline="\n") as fh:
        for i, r in enumerate(todo):
            audio, engine, voice, settings = speak(r["transcript"], r["id"])
            if not audio.size:
                print(f"{r['id']}: no audio", flush=True)
                continue
            audio = np.concatenate([pad, audio, pad])
            wav = f"{r['id']}.wav"
            sf.write(out / wav, audio, RATE, subtype="PCM_16")
            row = {
                "id": f"{args.collection}/{r['id']}",
                "rendition_id": r["id"],
                "audio": wav,
                "duration_s": round(len(audio) / RATE, 2),
                "language": "en",
                "transcript": r["transcript"],
                "final_text": r["final_text"],
                "category": r["category"],
                "domain": r["domain"],
                "annotations": r["annotations"],
                "tts_engine": engine,
                "tts_voice": voice,
                "tts_settings": settings,
                "collection": args.collection,
                "source": r["source"],
                "source_url": r["source_url"],
                "license": r["license"],
            }
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")
            fh.flush()
            if (i + 1) % 100 == 0:
                print(f"{i + 1}/{len(todo)}", flush=True)
    print(f"done -> {out}")


if __name__ == "__main__":
    main()
