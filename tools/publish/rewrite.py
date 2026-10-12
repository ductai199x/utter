"""Rewrites a built speech split by the rules both datasets share, from its own shards (no audio
re-encoding), re-sharded as voice.py shards, and writes its statistics:

- rows of excluded passages (publish.common.excluded_passages: license, wrapped-prose lists) go;
- final_text is the passage's current text (what the evals score); for a corpus clip, its
  dictation-to-text pair's final_text when that pair formats lists, so the two always agree;
- the corpus keeps exactly the renditions dictation-to-text has, each in its pair's split (the
  text dataset's held-out test passages are test audio too), under its corpus collection name.

Run (from tools): uv run python -m publish.rewrite OUT_DIR --split train --text TEXT_DIR
"""

from __future__ import annotations

import collections
import glob
import json
import shutil
from pathlib import Path

import pyarrow.parquet as pq
import typer

from publish.common import current_passages, excluded_passages, gold, passages
from publish.voice import CORPUS_NAMES, write_collection


def main(out: Path, split: str = typer.Option(...), text: Path | None = typer.Option(None)) -> None:
    X = excluded_passages(passages())
    C = current_passages()
    pairs = {}
    if text:
        for f in glob.glob(str(text / "data" / "*.parquet")):
            text_split = "test" if Path(f).name.startswith("test-") else "train"
            for r in pq.read_table(f, columns=["id", "final_text", "lists"]).to_pylist():
                r["split"] = text_split
                pairs[r["id"]] = r
    if split == "train" and not pairs:
        raise SystemExit(
            "the training split needs --text (dictation-to-text, to keep only its renditions)"
        )
    old_stats_path = out / f"stats-{split}.json"
    old_stats = json.loads(old_stats_path.read_text(encoding="utf-8"))
    stats = {}
    for old_name in old_stats:
        name = CORPUS_NAMES.get(old_name, old_name)
        files = sorted(glob.glob(str(out / "data" / old_name / f"{split}-*.parquet")))
        rows = [r for f in files for r in pq.read_table(f).to_pylist()]
        removed = collections.Counter()
        by_split: dict[str, list[dict]] = {"train": [], "test": []}
        for r in rows:
            pid = r["rendition_id"].split("-")[0]
            if pid in X:
                removed[X[pid]] += 1
                continue
            if split == "train":
                pair = pairs.get(r["rendition_id"])
                if pair is None:
                    removed["no text pair"] += 1
                    continue
                r["final_text"] = (
                    pair["final_text"] if pair["lists"] else gold(pid, r["final_text"], C)
                )
                r["collection"] = name
                r["id"] = f"{name}/{r['rendition_id']}"
                by_split[pair["split"]].append(r)
            else:
                r["final_text"] = gold(pid, r["final_text"], C)
                by_split["test"].append(r)
        s = write_collection(out, name, by_split)
        if name != old_name:
            shutil.rmtree(out / "data" / old_name, ignore_errors=True)
        prev = old_stats[old_name]
        s["dropped_as_in_train"] = prev.get("dropped_as_in_train", 0)
        s["excluded_for_license"] = prev.get("excluded_for_license", 0) + removed["license"]
        s["wrapped_prose_lists"] = (
            prev.get("wrapped_prose_lists", 0) + removed["wrapped-prose list"]
        )
        s["no_text_pair"] = prev.get("no_text_pair", 0) + removed["no text pair"]
        stats[name] = s
        print(name, dict(removed), s["split_rows"], flush=True)
    new_path = out / ("stats-corpus.json" if split == "train" else f"stats-{split}.json")
    old_stats_path.unlink()
    new_path.write_text(json.dumps(stats, indent=2), encoding="utf-8")


if __name__ == "__main__":
    typer.run(main)
