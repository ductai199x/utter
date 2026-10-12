"""Builds the text dataset (dictated -> final text pairs) as Parquet shards and its card, from the
latest training corpus: the Light v1 run's (train/e2b), which is every pair, with Dictionary
lines on about a third. Nothing is uploaded here (upload.py does that).

Run (from tools): uv run python -m publish.text OUT_DIR
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import pyarrow as pa
import typer

from eval.paths import data_dir
from publish.common import excluded_passages, fix_final, passages, provenance, shards, write_parquet

RUN = "e2b"
SPLITS = {"train": "pairs.jsonl", "test": "pairs.eval.jsonl"}

SCHEMA = pa.schema(
    [
        ("id", pa.string()),
        ("category", pa.string()),
        ("domain", pa.string()),
        ("dictated", pa.string()),
        ("final_text", pa.string()),
        ("dictionary", pa.list_(pa.string())),
        ("lists", pa.bool_()),
        ("messages", pa.list_(pa.struct([("role", pa.string()), ("content", pa.string())]))),
        ("source", pa.string()),
        ("source_url", pa.string()),
        ("license", pa.string()),
    ]
)
FEATURES = {
    "id": {"dtype": "string", "_type": "Value"},
    "category": {"dtype": "string", "_type": "Value"},
    "domain": {"dtype": "string", "_type": "Value"},
    "dictated": {"dtype": "string", "_type": "Value"},
    "final_text": {"dtype": "string", "_type": "Value"},
    "dictionary": {"feature": {"dtype": "string", "_type": "Value"}, "_type": "List"},
    "lists": {"dtype": "bool", "_type": "Value"},
    "messages": {
        "feature": {
            "role": {"dtype": "string", "_type": "Value"},
            "content": {"dtype": "string", "_type": "Value"},
        },
        "_type": "List",
    },
    "source": {"dtype": "string", "_type": "Value"},
    "source_url": {"dtype": "string", "_type": "Value"},
    "license": {"dtype": "string", "_type": "Value"},
}

_HEADER = re.compile(r"^((?:Dictionary: [^\n]*\n|Lists: off\n)*)Dictated: (.*)$", re.DOTALL)


def row(r: dict, P: dict) -> tuple[dict, bool]:
    system, user = r["prompt"]
    assistant = r["completion"][0]
    assert [m["role"] for m in (system, user, assistant)] == ["system", "user", "assistant"], r[
        "id"
    ]
    m = _HEADER.match(user["content"])
    assert m, f"{r['id']}: {user['content'][:80]!r}"
    header, dictated = m.groups()
    dictionary: list[str] = []
    for line in header.splitlines():
        if line.startswith("Dictionary: "):
            dictionary = [t.strip() for t in line[len("Dictionary: ") :].split(",") if t.strip()]
    final = fix_final(assistant["content"])
    p = P[r["id"].split("-")[0]]
    return {
        "id": r["id"],
        "category": r["kind"],
        "domain": p["domain"],
        "dictated": dictated,
        "final_text": final,
        "dictionary": dictionary,
        "lists": "Lists: off" not in header,
        # The exact chat the model was trained on, with the fixed final text.
        "messages": [
            {"role": "system", "content": system["content"]},
            {"role": "user", "content": user["content"]},
            {"role": "assistant", "content": final},
        ],
        **provenance(p),
    }, final != assistant["content"]


def main(out: Path) -> None:
    P = passages()
    X = excluded_passages(P)
    stats = {}
    for split, name in SPLITS.items():
        rows, order, fixed_ids, excluded = [], {}, {}, {"license": 0, "wrapped-prose list": 0}
        for line in (data_dir() / "train" / RUN / name).open(encoding="utf-8"):
            raw = json.loads(line)
            why = X.get(raw["id"].split("-")[0])
            if why:
                excluded[why] += 1
                continue
            built, was_fixed = row(raw, P)
            fixed_ids[built["id"]] = was_fixed
            order[built["id"]] = len(rows)
            rows.append(built)
        assert len({r["id"] for r in rows}) == len(rows), f"{split}: duplicate ids"
        # Two renditions of one passage that the recognizer heard identically (say "natural" and
        # "explicit") make the same pair twice: one is kept, natural first, so no pair repeats and
        # no row carries a category its input does not show.
        rows.sort(key=lambda r: (r["id"].split("-")[0], r["category"] != "natural", r["id"]))
        kept, pairs = [], set()
        for r in rows:
            key = (r["dictated"], r["final_text"])
            if key not in pairs:
                pairs.add(key)
                kept.append(r)
        duplicates = len(rows) - len(kept)
        rows = sorted(kept, key=lambda r: order[r["id"]])
        parts = shards(rows, 300 << 20, lambda r: sum(len(m["content"]) for m in r["messages"]) * 2)
        for i, part in enumerate(parts):
            path = out / "data" / f"{split}-{i:05d}-of-{len(parts):05d}.parquet"
            write_parquet(
                path, pa.Table.from_pylist(part, schema=SCHEMA), FEATURES, row_group_size=10_000
            )
        stats[split] = {
            "rows": len(rows),
            "fixed": sum(fixed_ids.get(r["id"], 0) for r in rows),
            "duplicates_removed": duplicates,
            "excluded_for_license": excluded["license"],
            "wrapped_prose_lists": excluded["wrapped-prose list"],
            "shards": len(parts),
        }
        print(split, stats[split])
    (out / "stats.json").write_text(json.dumps(stats, indent=2), encoding="utf-8")


if __name__ == "__main__":
    typer.run(main)
