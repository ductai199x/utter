"""Fetch Utter's speech-recognition eval sets: a fixed random sample of each public test set.

Each set becomes `<eval_dir>/<name>/` with 16 kHz mono PCM16 WAVs and a `manifest.jsonl`
(one object per clip: id, audio, text, lang, duration_s, source). The sample is seeded, so a
re-run produces the same clips; a set already complete is skipped.

Parquet sets are read a row group at a time over the Hub's file system, so only the sampled
row groups are transferred, not whole shards. Rows come from several row groups, spread over
the shards, so one meeting or one speaker does not dominate a set.

Run: uv run python -m eval.fetch_sets [NAMES...] [--per-set 300]
"""

from __future__ import annotations

import io
import json
import math
import random
import tarfile
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq
import soundfile as sf
import soxr
import typer
from huggingface_hub import HfFileSystem, hf_hub_download
from rich.console import Console

from eval.paths import eval_dir

SAMPLE_RATE = 16_000
# Dictation-sized clips: long enough to carry a sentence, short enough to be one utterance.
MIN_S, MAX_S = 1.0, 30.0
# How many row groups a parquet set's sample is drawn from.
ROW_GROUPS_PER_SET = 8

console = Console()
app = typer.Typer(add_completion=False)


@dataclass(frozen=True)
class Clip:
    id: str
    audio: bytes  # an encoded audio file (wav, flac, mp3, ...)
    text: str


@dataclass(frozen=True)
class ParquetSet:
    name: str
    lang: str
    repo: str  # "datasets/<owner>/<name>"
    glob: str  # shard pattern inside the repo
    text_col: str
    id_col: str | None  # None: the id is made from the shard and row
    note: str


@dataclass(frozen=True)
class TarSet:
    name: str
    lang: str
    repo: str
    tar: str
    tsv: str
    note: str


SETS: list[ParquetSet | TarSet] = [
    # English: the Open ASR Leaderboard's own test sets (hf-audio/open-asr-leaderboard).
    ParquetSet("en-ami", "en", "datasets/hf-audio/open-asr-leaderboard", "ami_cleaned/test-*.parquet",
               "text", "id", "meetings, spontaneous, disfluent"),
    ParquetSet("en-earnings22", "en", "datasets/hf-audio/open-asr-leaderboard", "earnings22/test-*.parquet",
               "text", "id", "earnings calls, accented, spontaneous"),
    ParquetSet("en-gigaspeech", "en", "datasets/hf-audio/open-asr-leaderboard", "gigaspeech_cleaned/test-*.parquet",
               "text", "id", "podcasts and YouTube"),
    ParquetSet("en-voxpopuli", "en", "datasets/hf-audio/open-asr-leaderboard", "voxpopuli_cleaned_aa/test-*.parquet",
               "text", "id", "parliament speeches"),
    ParquetSet("en-urgent24", "en", "datasets/hf-audio/open-asr-leaderboard", "urgent2024/test-*.parquet",
               "text", "id", "noise and reverberation added (research use only)"),
    ParquetSet("en-monsoon", "en", "datasets/VoiceArena/Monsoon_en_IN_test", "data/test-*.parquet",
               "text", "id", "conversations, Indian English"),
    ParquetSet("en-libri-other", "en", "datasets/hf-audio/open-asr-leaderboard", "librispeech/test.other-*.parquet",
               "text", "id", "read audiobooks, the harder half (a sanity baseline)"),
    # Vietnamese.
    ParquetSet("vi-vimd", "vi", "datasets/nguyendv02/ViMD_Dataset", "data/test-*.parquet",
               "text", "filename", "63 provincial dialects"),
    ParquetSet("vi-fleurs", "vi", "datasets/google/fleurs", "parquet-data/vi_vn/test-*.parquet",
               "raw_transcription", "id", "read Wikipedia sentences"),
    ParquetSet("vi-bud500", "vi", "datasets/linhtran92/viet_bud500", "data/test-*.parquet",
               "transcription", None, "podcasts and more (gated)"),
    TarSet("vi-gigaspeech2", "vi", "speechcolab/gigaspeech2", "data/vi/test.tar.gz", "data/vi/test.tsv",
           "YouTube and podcasts, in the wild (gated)"),
]


def to_16k_mono(encoded: bytes) -> np.ndarray:
    data, rate = sf.read(io.BytesIO(encoded), dtype="float32", always_2d=True)
    mono = data.mean(axis=1)
    if rate != SAMPLE_RATE:
        mono = soxr.resample(mono, rate, SAMPLE_RATE, quality="VHQ")
    return mono.astype(np.float32)


def parquet_clips(spec: ParquetSet, per_set: int, rng: random.Random) -> Iterator[Clip]:
    fs = HfFileSystem()
    shards = sorted(fs.glob(f"{spec.repo}/{spec.glob}"))
    if not shards:
        raise RuntimeError(f"{spec.name}: no shards match {spec.glob}")
    groups: list[tuple[str, int]] = []
    for shard in shards:
        with fs.open(shard, "rb") as f:
            groups += [(shard, g) for g in range(pq.ParquetFile(f).metadata.num_row_groups)]
    rng.shuffle(groups)
    chosen = groups[:ROW_GROUPS_PER_SET]
    # Ask each row group for a little more than its share: some rows fail the length filter.
    share = math.ceil(per_set * 1.5 / len(chosen))
    for shard, g in chosen:
        with fs.open(shard, "rb", block_size=16 * 2**20) as f:
            table = pq.ParquetFile(f).read_row_group(g)
        rows = table.to_pylist()
        rng.shuffle(rows)
        for i, row in enumerate(rows[:share]):
            audio = row["audio"]["bytes"]
            text = (row.get(spec.text_col) or "").strip()
            if not audio or not text:
                continue
            ident = str(row[spec.id_col]) if spec.id_col else f"{Path(shard).stem}-{g}-{i}"
            yield Clip(ident, audio, text)


def tar_clips(spec: TarSet, per_set: int, rng: random.Random) -> Iterator[Clip]:
    tsv = hf_hub_download(spec.repo, spec.tsv, repo_type="dataset")
    texts: dict[str, str] = {}
    for line in Path(tsv).read_text(encoding="utf-8").splitlines():
        ident, _, text = line.partition("\t")
        if text.strip():
            texts[ident] = text.strip()
    wanted = set(rng.sample(sorted(texts), min(len(texts), math.ceil(per_set * 1.5))))
    archive = hf_hub_download(spec.repo, spec.tar, repo_type="dataset")
    with tarfile.open(archive, "r:gz") as tar:
        for member in tar:
            if not member.isfile():
                continue
            ident = Path(member.name).stem
            if ident in wanted:
                f = tar.extractfile(member)
                if f is not None:
                    yield Clip(ident, f.read(), texts[ident])


def fetch(spec: ParquetSet | TarSet, per_set: int) -> None:
    out = eval_dir() / spec.name
    manifest = out / "manifest.jsonl"
    if manifest.exists() and sum(1 for _ in manifest.open(encoding="utf-8")) >= per_set:
        console.print(f"[dim]{spec.name}: already has {per_set} clips, skipped[/dim]")
        return
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(f"utter-eval-{spec.name}")
    source: Callable[[], Iterator[Clip]] = (
        (lambda: parquet_clips(spec, per_set, rng)) if isinstance(spec, ParquetSet)
        else (lambda: tar_clips(spec, per_set, rng))
    )
    kept: list[dict] = []
    skipped = 0
    with console.status(f"{spec.name}: fetching"):
        for clip in source():
            try:
                audio = to_16k_mono(clip.audio)
            except Exception as e:  # noqa: BLE001 -- one undecodable clip must not end the set
                console.print(f"[yellow]{spec.name}: {clip.id} not decodable ({e})[/yellow]")
                skipped += 1
                continue
            duration = len(audio) / SAMPLE_RATE
            if not MIN_S <= duration <= MAX_S:
                skipped += 1
                continue
            safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in clip.id)
            path = out / f"{safe}.wav"
            sf.write(path, audio, SAMPLE_RATE, subtype="PCM_16")
            kept.append({"id": clip.id, "audio": path.name, "text": clip.text, "lang": spec.lang,
                         "duration_s": round(duration, 3), "source": spec.name})
            if len(kept) >= per_set:
                break
    tmp = manifest.with_suffix(".jsonl.part")
    with tmp.open("w", encoding="utf-8", newline="\n") as f:
        for row in kept:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
    tmp.replace(manifest)
    hours = sum(r["duration_s"] for r in kept) / 3600
    console.print(f"{spec.name}: {len(kept)} clips, {hours:.2f} h ({skipped} outside {MIN_S:g}-{MAX_S:g} s or undecodable)"
                  + ("" if len(kept) >= per_set else f" [yellow]-- short of {per_set}[/yellow]"))


@app.command()
def main(
    names: list[str] = typer.Argument(None, help="Sets to fetch (default: all)."),
    per_set: int = typer.Option(300, help="Clips per set."),
) -> None:
    by_name = {s.name: s for s in SETS}
    unknown = [n for n in names or [] if n not in by_name]
    if unknown:
        raise typer.BadParameter(f"unknown sets {unknown}; known: {sorted(by_name)}")
    console.print(f"Eval data folder: {eval_dir()}")
    for spec in [by_name[n] for n in names] if names else SETS:
        try:
            fetch(spec, per_set)
        except Exception as e:  # noqa: BLE001 -- report and go on to the next set
            console.print(f"[red]{spec.name}: failed: {type(e).__name__}: {e}[/red]")


if __name__ == "__main__":
    app()
