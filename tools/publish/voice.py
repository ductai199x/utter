"""Builds one split of the speech dataset (dictation audio -> transcript and final text) as Parquet
shards, from Utter's synthesized sets: each collection's manifest, its audio (stored as FLAC,
lossless), and the source passage's provenance. Run once per split where that split's audio lives
(the test sets on the development machine, the training sets on FLAME); upload.py uploads.

Run (from tools): uv run python -m publish.voice OUT_DIR --split test
"""

from __future__ import annotations

import collections
import io
import json
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import soundfile as sf
import typer

from eval.paths import eval_dir
from publish.common import (
    current_passages,
    excluded_passages,
    gold,
    passages,
    provenance,
    write_parquet,
)

# collection -> (manifest folder, folders its audio may be in)
TEST = {
    c: (f"en-finaltext-{c}", [f"en-finaltext-{c}"])
    for c in (
        "kokoro",
        "kokoro-amend",
        "kokoro-directed",
        "kokoro-grammar",
        "kokoro-lists",
        "kokoro-real",
        "kokoro-restate",
        "chatterbox",
        "chatterbox-real",
    )
}
# The training corpus's audio, named by content (its clips are in both splits, as their
# dictation-to-text pairs are).
TRAIN = {
    "corpus": (
        "en-finaltext-train-kokoro",
        [
            "en-finaltext-train-kokoro",
            "en-finaltext-train-kokoro2",
            "en-finaltext-train-kokoro-respoken",
        ],
    ),
    **{
        f"corpus-{c}": (f"en-finaltext-train-kokoro-{c}", [f"en-finaltext-train-kokoro-{c}"])
        for c in ("v2", "messages", "amend", "grammar", "grammar2", "lists")
    },
}
# Earlier builds' names for the corpus collections (publish.rewrite renames them).
CORPUS_NAMES = {
    "train-kokoro": "corpus",
    **{
        f"train-kokoro-{c}": f"corpus-{c}"
        for c in ("v2", "messages", "amend", "grammar", "grammar2", "lists")
    },
}
# Audio spoken again because the original was lost (tools/tts/kokoro/respeak.py).
RESPOKEN = "en-finaltext-train-kokoro-respoken"
RATE = 16_000
SHARD_BYTES = 500 << 20
ROW_GROUP = 100

SCHEMA = pa.schema(
    [
        ("id", pa.string()),
        ("rendition_id", pa.string()),
        ("audio", pa.struct([("bytes", pa.binary()), ("path", pa.string())])),
        ("duration_s", pa.float32()),
        ("language", pa.string()),
        ("transcript", pa.string()),
        ("final_text", pa.string()),
        ("category", pa.string()),
        ("domain", pa.string()),
        ("annotations", pa.list_(pa.string())),
        ("tts_engine", pa.string()),
        ("tts_voice", pa.string()),
        ("tts_settings", pa.list_(pa.string())),
        ("collection", pa.string()),
        ("source", pa.string()),
        ("source_url", pa.string()),
        ("license", pa.string()),
    ]
)


def _value(dtype: str) -> dict:
    return {"dtype": dtype, "_type": "Value"}


FEATURES = {
    "id": _value("string"),
    "rendition_id": _value("string"),
    "audio": {"sampling_rate": RATE, "_type": "Audio"},
    "duration_s": _value("float32"),
    "language": _value("string"),
    "transcript": _value("string"),
    "final_text": _value("string"),
    "category": _value("string"),
    "domain": _value("string"),
    "annotations": {"feature": _value("string"), "_type": "List"},
    "tts_engine": _value("string"),
    "tts_voice": _value("string"),
    "tts_settings": {"feature": _value("string"), "_type": "List"},
    "collection": _value("string"),
    "source": _value("string"),
    "source_url": _value("string"),
    "license": _value("string"),
}


def donor_voices() -> dict[str, str]:
    """Chatterbox's reference voices, numbered (donors' names stay out of the data)."""
    names: set[str] = set()
    for folder, _ in TEST.values():
        m = eval_dir() / folder / "manifest.jsonl"
        if folder.startswith("en-finaltext-chatterbox") and m.is_file():
            names |= {json.loads(line)["voice"] for line in m.open(encoding="utf-8")}
    return {n: f"donated-voice-{i + 1:02d}" for i, n in enumerate(sorted(names))}


def flac(path: Path) -> tuple[bytes, float]:
    audio, rate = sf.read(path, dtype="int16", always_2d=True)
    if rate != RATE or audio.shape[1] != 1:
        raise ValueError(f"{path}: {rate} Hz, {audio.shape[1]} channels; expected 16 kHz mono")
    buf = io.BytesIO()
    sf.write(buf, audio[:, 0], RATE, format="FLAC", subtype="PCM_16")
    return buf.getvalue(), len(audio) / RATE


def write_collection(out: Path, collection: str, by_split: dict[str, list[dict]]) -> dict:
    """Writes one collection's rows, each split into its own shards of about SHARD_BYTES of audio,
    in order; returns its statistics."""
    import shutil

    target = out / "data" / collection
    tmp = out / "data" / f"{collection}.new"
    shutil.rmtree(tmp, ignore_errors=True)
    split_rows = {}
    for split, rows in by_split.items():
        if not rows:
            continue
        parts, size = [[]], 0
        for r in rows:
            n = len(r["audio"]["bytes"])
            if parts[-1] and size + n > SHARD_BYTES:
                parts.append([])
                size = 0
            parts[-1].append(r)
            size += n
        for i, part in enumerate(parts):
            write_parquet(
                tmp / f"{split}-{i:05d}-of-{len(parts):05d}.parquet",
                pa.Table.from_pylist(part, schema=SCHEMA),
                FEATURES,
                ROW_GROUP,
                binary_columns=["audio.bytes"],
            )
        split_rows[split] = len(rows)
    shutil.rmtree(target, ignore_errors=True)
    tmp.rename(target)
    rows = [r for rs in by_split.values() for r in rs]
    return {
        "split_rows": split_rows,
        "rows": len(rows),
        "hours": round(sum(r["duration_s"] for r in rows) / 3600, 2),
        "regenerated": sum("audio=regenerated" in r["tts_settings"] for r in rows),
        "categories": sorted({r["category"] for r in rows}),
    }


def main(
    out: Path,
    split: str = typer.Option(
        ...,
        help="test (the eval sets) or train (the corpus: each clip goes to its text pair's split)",
    ),
    exclude: Path | None = None,
    manifests: Path | None = typer.Option(
        None, help="Read <folder>.jsonl here instead of each folder's manifest.jsonl"
    ),
    text: Path | None = typer.Option(
        None,
        help="dictation-to-text's folder: the training split keeps its renditions, with its final texts",
    ),
) -> None:
    sets = TEST if split == "test" else TRAIN
    P = passages()
    C = current_passages()
    X = excluded_passages(P)
    pairs = {}
    if text:
        for f in sorted(text.glob("data/*.parquet")):
            for t in pq.read_table(f, columns=["id", "final_text", "lists"]).to_pylist():
                t["split"] = "test" if f.name.startswith("test-") else "train"
                pairs[t["id"]] = t
    if split == "train" and not pairs:
        raise SystemExit("the training split needs --text")
    voices = donor_voices()
    # Renditions also in the training sets are left out of test (no leakage).
    skip = set(json.loads(exclude.read_text())) if exclude else set()
    stats: dict[str, dict] = {}
    seen: set[str] = set()
    for collection, (folder, audio_dirs) in sets.items():
        manifest = (
            manifests / f"{folder}.jsonl" if manifests else eval_dir() / folder / "manifest.jsonl"
        )
        rows = [json.loads(line) for line in manifest.open(encoding="utf-8")]
        built, dropped, excluded, hours = [], 0, collections.Counter(), 0.0
        for r in rows:
            if r["id"] in skip:
                dropped += 1
                continue
            pid = r["id"].split("-")[0]
            if pid in X:
                excluded[X[pid]] += 1
                continue
            pair = pairs.get(r["id"])
            if split == "train" and pair is None:
                excluded["no text pair"] += 1
                continue
            src = next(
                (
                    eval_dir() / d / r["audio"]
                    for d in audio_dirs
                    if (eval_dir() / d / r["audio"]).is_file()
                ),
                None,
            )
            if src is None:
                raise FileNotFoundError(f"{collection}: no audio for {r['id']}")
            data, seconds = flac(src)
            hours += seconds / 3600
            p = P[r["id"].split("-")[0]]
            engine = r["tts"]
            if engine.startswith("chatterbox"):
                voice = voices[r["voice"]]
                settings = [f"exaggeration={r['exaggeration']}"]
            else:
                voice = r["voice"]
                settings = [f"speed={r['speed']}"]
            if src.parent.name == RESPOKEN:
                settings.append("audio=regenerated")
            row = {
                "id": f"{collection}/{r['id']}",
                "rendition_id": r["id"],
                "audio": {"bytes": data, "path": f"{r['id']}.flac"},
                "duration_s": round(seconds, 2),
                "language": r["lang"],
                "transcript": r["text"],
                # What the evals score and the training used: the passage's current text, or (a
                # training clip) its dictation-to-text pair's when that pair formats lists.
                "final_text": pair["final_text"]
                if pair and pair["lists"]
                else gold(pid, r["gold"], C),
                "category": r["kind"],
                "domain": r["domain"],
                "annotations": [f"{k}={v}" for k, v in r["tags"].items()],
                "tts_engine": engine,
                "tts_voice": voice,
                "tts_settings": settings,
                "collection": collection,
                **provenance(p),
            }
            assert row["id"] not in seen, row["id"]
            seen.add(row["id"])
            assert all(v not in (None, "") for k, v in row.items() if k not in ("annotations",)), (
                row["id"]
            )
            built.append(row)
        by_split: dict[str, list[dict]] = {"train": [], "test": []}
        for row in built:
            # A corpus clip goes where its text pair is: the text dataset's held-out passages are
            # test audio too.
            by_split[pairs[row["rendition_id"]]["split"] if split == "train" else "test"].append(
                row
            )
        s_ = write_collection(out, collection, by_split)
        s_.update(
            dropped_as_in_train=dropped,
            excluded_for_license=excluded["license"],
            wrapped_prose_lists=excluded["wrapped-prose list"],
            no_text_pair=excluded["no text pair"],
        )
        stats[collection] = s_
        print(collection, s_["split_rows"], flush=True)
    name = "stats-corpus.json" if split == "train" else f"stats-{split}.json"
    (out / name).write_text(json.dumps(stats, indent=2), encoding="utf-8")


if __name__ == "__main__":
    typer.run(main)
