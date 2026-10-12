"""Builds the text dataset (dictated -> final text pairs) as Parquet shards and its card, from the
latest training corpus: the Light v1 run's (train/e2b), which is every pair, with Dictionary
lines on about a third. Nothing is uploaded here (upload.py does that).

Run (from tools): uv run python -m publish.text OUT_DIR
"""

from __future__ import annotations

import collections
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


# The share of rows in test (about 10,000 of the corpus's 89,000).
TEST_SHARE = 0.112


def _length_tercile(n: int, cuts: tuple[int, int]) -> str:
    return "short" if n <= cuts[0] else "medium" if n <= cuts[1] else "long"


def split_by_passage(rows: list[dict]) -> dict[str, str]:
    """Passage id -> "train" or "test". A passage's renditions stay together (else test would hold
    near-copies of training rows). Passages are grouped by everything that tells them apart (source, license,
    domain, which categories they are spoken in, length, whether any rendition has a dictionary or
    lists off), and each group gives TEST_SHARE of its rows to test, in a fixed hash order, so every
    aspect is balanced between the splits."""
    import hashlib

    by_passage: dict[str, list[dict]] = collections.defaultdict(list)
    for r in rows:
        by_passage[r["id"].split("-")[0]].append(r)
    lengths = sorted(len(rs[0]["final_text"]) for rs in by_passage.values())
    cuts = (lengths[len(lengths) // 3], lengths[2 * len(lengths) // 3])
    groups: dict[tuple, list[str]] = collections.defaultdict(list)
    for pid, rs in by_passage.items():
        key = (
            rs[0]["source"],
            rs[0]["license"],
            rs[0]["domain"],
            tuple(sorted({r["category"] for r in rs})),
            _length_tercile(len(rs[0]["final_text"]), cuts),
            any(r["dictionary"] for r in rs),
            any(not r["lists"] for r in rs),
        )
        groups[key].append(pid)
    split = {}
    carry = 0.0  # rows owed to test across groups too small to give a whole passage
    for key in sorted(groups):
        pids = sorted(groups[key], key=lambda p: hashlib.sha256(p.encode()).hexdigest())
        want = TEST_SHARE * sum(len(by_passage[p]) for p in pids) + carry
        taken = 0
        for pid in pids:
            n = len(by_passage[pid])
            if taken + n / 2 <= want:
                split[pid] = "test"
                taken += n
            else:
                split[pid] = "train"
        carry = want - taken
    return split


def balance(rows: list[dict], split: dict[str, str]) -> dict[str, dict[str, float]]:
    """Per aspect and value: the share of its rows in test."""
    lengths = sorted(len(r["final_text"]) for r in rows)
    cuts = (lengths[len(lengths) // 3], lengths[2 * len(lengths) // 3])
    aspects = {
        "category": lambda r: r["category"],
        "domain": lambda r: r["domain"],
        "source": lambda r: r["source"],
        "license": lambda r: r["license"],
        "dictionary": lambda r: "with" if r["dictionary"] else "without",
        "lists": lambda r: "on" if r["lists"] else "off",
        "length": lambda r: _length_tercile(len(r["final_text"]), cuts),
    }
    out = {}
    for name, f in aspects.items():
        total, test = collections.Counter(), collections.Counter()
        for r in rows:
            v = f(r)
            total[v] += 1
            test[v] += split[r["id"].split("-")[0]] == "test"
        out[name] = {v: round(test[v] / total[v], 4) for v in total if total[v]}
        out[name]["_rows"] = dict(total)
    return out


def main(out: Path) -> None:
    P = passages()
    X = excluded_passages(P)
    # The whole corpus, both of the training run's files, split afresh below.
    rows, order, fixed, excluded = [], {}, 0, collections.Counter()
    for name in SPLITS.values():
        for line in (data_dir() / "train" / RUN / name).open(encoding="utf-8"):
            raw = json.loads(line)
            why = X.get(raw["id"].split("-")[0])
            if why:
                excluded[why] += 1
                continue
            built, was_fixed = row(raw, P)
            order[built["id"]] = len(rows)
            rows.append(built)
            fixed += was_fixed
    assert len({r["id"] for r in rows}) == len(rows), "duplicate ids"
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
    split = split_by_passage(rows)
    stats = {}
    for name in ("train", "test"):
        part_rows = [r for r in rows if split[r["id"].split("-")[0]] == name]
        parts = shards(
            part_rows, 300 << 20, lambda r: sum(len(m["content"]) for m in r["messages"]) * 2
        )
        for i, part in enumerate(parts):
            path = out / "data" / f"{name}-{i:05d}-of-{len(parts):05d}.parquet"
            write_parquet(
                path, pa.Table.from_pylist(part, schema=SCHEMA), FEATURES, row_group_size=10_000
            )
        stats[name] = {"rows": len(part_rows), "shards": len(parts)}
        print(name, stats[name])
    stats["build"] = {
        "fixed": fixed,
        "duplicates_removed": duplicates,
        "excluded": dict(excluded),
        "test_share": TEST_SHARE,
    }
    stats["balance"] = balance(rows, split)
    (out / "stats.json").write_text(json.dumps(stats, indent=2), encoding="utf-8")


if __name__ == "__main__":
    typer.run(main)
