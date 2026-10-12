"""Publishing a batch: its rows merged into the two datasets, audited, then uploaded.

`build` writes, under `<data dir>/data/<batch>/publish/`:
  text/     the whole text dataset: the Hub's rows at its revision plus the batch's, split by
            passage (published passages keep their split; the rest are split as
            schema.split_by_passage balances them), as Parquet shards, and its card. An eval
            batch (test-only) has none: eval sets are speech only, as the published ones are;
  speech/   the batch's collection as Parquet shards (FLAC audio, lossless), each clip in its text
            row's split, and the speech dataset's card covering every shard (the Hub's, summarized
            remotely, and these);
  build.json  the revisions merged and the counts.
`upload` sends both, only after the audit passes: the text dataset's shards are replaced, the
speech dataset gains the batch's collection.
"""

from __future__ import annotations

import io
import json
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import soundfile as sf
from huggingface_hub import HfApi, HfFileSystem

from utter_tools import config, hub, schema
from utter_tools.data import card
from utter_tools.data.batch import Batch, read_jsonl


@dataclass(frozen=True)
class Layout:
    """How a dataset's rows are written: Hub file names, shards of about `shard_bytes` (as
    `size_of` counts a row), and row groups of `row_group` rows."""

    table_schema: pa.Schema
    features: dict
    shard_bytes: int
    row_group: int
    size_of: Callable[[dict], int]
    binary: tuple[str, ...] = ()

    def write(self, rows: list[dict], target: Path, split: str) -> None:
        parts = schema.shards(rows, self.shard_bytes, self.size_of)
        for i, part in enumerate(parts):
            schema.write_parquet(
                target / f"{split}-{i:05d}-of-{len(parts):05d}.parquet",
                pa.Table.from_pylist(part, schema=self.table_schema),
                self.features,
                self.row_group,
                binary_columns=self.binary,
            )


TEXT = Layout(
    schema.TEXT_SCHEMA,
    schema.TEXT_FEATURES,
    300 << 20,
    10_000,
    lambda r: sum(len(m["content"]) for m in r["messages"]) * 2,
)
# Audio is stored as it is (FLAC, already compressed), as the datasets library writes it.
SPEECH = Layout(
    schema.SPEECH_SCHEMA,
    schema.SPEECH_FEATURES,
    500 << 20,
    100,
    lambda r: len(r["audio"]["bytes"]),
    ("audio.bytes",),
)


def folder(batch: Batch) -> Path:
    return batch.root / "publish"


def build_text(batch: Batch, out: Path, rev: str) -> dict[str, str]:
    """The merged text dataset in `out`; returns passage id -> split."""
    published = {s: hub.text_rows(config.TEXT_REPO, rev, s) for s in ("train", "test")}
    fixed = {schema.passage_id(r["id"]): s for s, rows in published.items() for r in rows}
    new = read_jsonl(batch.text_path)
    if not new:
        raise ValueError(f"batch {batch.meta.name} has no text rows: utter-data pairs")
    clash = {r["id"] for r in new} & {r["id"] for rows in published.values() for r in rows}
    if clash:
        raise ValueError(f"{len(clash)} batch rows already published, e.g. {sorted(clash)[:2]}")
    rows = published["train"] + published["test"] + new
    split = schema.split_by_passage(rows, fixed)
    for part in ("train", "test"):
        TEXT.write(
            [r for r in rows if split[schema.passage_id(r["id"])] == part], out / "data", part
        )
    card.text(out)
    return split


def _flac(wav: Path) -> tuple[bytes, float]:
    audio, rate = sf.read(wav, dtype="int16", always_2d=True)
    if rate != schema.RATE or audio.shape[1] != 1:
        raise ValueError(f"{wav}: {rate} Hz, {audio.shape[1]} channels; expected 16 kHz mono")
    buf = io.BytesIO()
    sf.write(buf, audio[:, 0], rate, format="FLAC", subtype="PCM_16")
    return buf.getvalue(), len(audio) / rate


def speech_rows(batch: Batch, text_ids: set[str]) -> list[dict]:
    """The batch's clips as speech rows with their audio. A corpus clip is kept only if its text
    row is (one clip per text row); a test-only batch keeps every clip."""
    rows = []
    for clip in read_jsonl(batch.clips / "manifest.jsonl"):
        if not batch.meta.test_only and clip["rendition_id"] not in text_ids:
            continue
        data, seconds = _flac(batch.clips / clip["audio"])
        rows.append(
            {
                **clip,
                "audio": {"bytes": data, "path": f"{clip['rendition_id']}.flac"},
                "duration_s": round(seconds, 2),
            }
        )
    return rows


def hub_summaries(rev: str) -> dict[str, dict]:
    """Summaries of the speech dataset's shards on the Hub, reading only their small columns."""
    fs = HfFileSystem()
    root = f"datasets/{config.SPEECH_REPO}@{rev}/data"
    out = {}
    for path in sorted(fs.glob(f"{root}/*/*.parquet")):
        collection, name = path.split("/")[-2:]
        table = pq.read_table(path, columns=card.SUMMARY_COLUMNS, filesystem=fs)
        out[f"{collection}/{name}"] = card.shard_summary(
            table, name.split("-")[0], collection, int(fs.size(path) or 0)
        )
    return out


def build_speech(
    batch: Batch, out: Path, rev: str, split: dict[str, str], text_ids: set[str]
) -> int:
    rows = speech_rows(batch, text_ids)
    target = out / "data" / batch.meta.collection
    for part in ("train", "test"):
        mine = [r for r in rows if split[schema.passage_id(r["rendition_id"])] == part]
        if mine:
            SPEECH.write(mine, target, part)
    remote = hub_summaries(rev)
    # Kept for the audit: the card's figures cover these shards too.
    (out / "hub-shards.json").write_text(json.dumps(remote, indent=1), encoding="utf-8")
    card.speech(out, remote | card.local_summaries(out))
    return len(rows)


def build(batch: Batch, text_ref: str | None = None, speech_ref: str | None = None) -> dict:
    out = folder(batch)
    text_rev = hub.revision(config.TEXT_REPO, text_ref)
    speech_rev = hub.revision(config.SPEECH_REPO, speech_ref)
    if batch.meta.test_only:
        split = {schema.passage_id(r["id"]): "test" for r in read_jsonl(batch.renditions_path)}
        text_ids: set[str] = set()
    else:
        split = build_text(batch, out / "text", text_rev)
        text_ids = {r["id"] for r in read_jsonl(batch.text_path)}
    clips = build_speech(batch, out / "speech", speech_rev, split, text_ids)
    info = {
        "batch": batch.meta.name,
        "collection": batch.meta.collection,
        config.TEXT_REPO: text_rev,
        config.SPEECH_REPO: speech_rev,
        "text_rows_added": len(text_ids),
        "clips_added": clips,
    }
    (out / "build.json").write_text(json.dumps(info, indent=2), encoding="utf-8")
    return info


def build_info(batch: Batch) -> dict:
    """What `build` merged (revisions and counts)."""
    path = folder(batch) / "build.json"
    if not path.is_file():
        raise FileNotFoundError(f"batch {batch.meta.name} is not built: utter-data build")
    return json.loads(path.read_text())


def upload(batch: Batch) -> None:
    """Uploads a built batch (the caller has the audit's verdict and the person's go-ahead)."""
    out = folder(batch)
    info = build_info(batch)
    api = HfApi()
    for repo in (config.TEXT_REPO, config.SPEECH_REPO):
        if hub.revision(repo) != info[repo]:
            raise RuntimeError(f"{repo} changed since the build: build again")
    if not batch.meta.test_only:
        api.upload_folder(
            repo_id=config.TEXT_REPO,
            repo_type="dataset",
            folder_path=out / "text",
            allow_patterns=["README.md", "data/*.parquet"],
            delete_patterns=["data/*.parquet"],
            commit_message=f"Add batch {batch.meta.name}",
        )
    api.upload_folder(
        repo_id=config.SPEECH_REPO,
        repo_type="dataset",
        folder_path=out / "speech",
        allow_patterns=["README.md", f"data/{batch.meta.collection}/*.parquet"],
        commit_message=f"Add collection {batch.meta.collection} (batch {batch.meta.name})",
    )
