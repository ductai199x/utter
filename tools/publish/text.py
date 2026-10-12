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
from publish.common import fix_final, passages, provenance, shards, write_parquet

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
        "feature": {"role": {"dtype": "string", "_type": "Value"}, "content": {"dtype": "string", "_type": "Value"}},
        "_type": "List",
    },
    "source": {"dtype": "string", "_type": "Value"},
    "source_url": {"dtype": "string", "_type": "Value"},
    "license": {"dtype": "string", "_type": "Value"},
}

_HEADER = re.compile(r"^((?:Dictionary: [^\n]*\n|Lists: off\n)*)Dictated: (.*)$", re.S)


def row(r: dict, P: dict) -> tuple[dict, bool]:
    system, user = r["prompt"]
    assistant = r["completion"][0]
    assert [m["role"] for m in (system, user, assistant)] == ["system", "user", "assistant"], r["id"]
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
    stats = {}
    for split, name in SPLITS.items():
        rows, fixed = [], 0
        for line in (data_dir() / "train" / RUN / name).open(encoding="utf-8"):
            built, was_fixed = row(json.loads(line), P)
            rows.append(built)
            fixed += was_fixed
        assert len({r["id"] for r in rows}) == len(rows), f"{split}: duplicate ids"
        parts = shards(rows, 300 << 20, lambda r: sum(len(m["content"]) for m in r["messages"]) * 2)
        for i, part in enumerate(parts):
            path = out / "data" / f"{split}-{i:05d}-of-{len(parts):05d}.parquet"
            write_parquet(path, pa.Table.from_pylist(part, schema=SCHEMA), FEATURES, row_group_size=10_000)
        stats[split] = {"rows": len(rows), "fixed": fixed, "shards": len(parts)}
        print(split, stats[split])
    (out / "stats.json").write_text(json.dumps(stats, indent=2), encoding="utf-8")


if __name__ == "__main__":
    typer.run(main)
