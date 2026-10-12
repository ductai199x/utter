"""What both published datasets share: where the passages come from (source, URL, license),
license ids, the mechanical fixes to final texts, and writing Parquet the way the Hub reads it
(docs: huggingface.co/docs/hub/datasets-*, and the datasets library's own writer settings)."""

from __future__ import annotations

import glob
import json
import re
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

from eval.paths import data_dir

REPO_BLOB = "https://github.com/ductai199x/utter/blob/v0.1.0"
# What wrote the synthetic passages, by domain (tools/eval/finaltext).
SYNTHETIC_SCRIPT = {
    "list": f"{REPO_BLOB}/tools/eval/finaltext/lists.py",
}
SYNTHETIC_DEFAULT = f"{REPO_BLOB}/tools/eval/finaltext/messages.py"
# The dataset's license: share-alike, as most of its text requires (Stack Exchange, Wikipedia);
# text Utter's tools wrote takes it too.
DATASET_LICENSE = "cc-by-sa-4.0"


def passages() -> dict[str, dict]:
    """Every passage by id, from the current passage files (not the .pre-/.raw/.v1 snapshots)."""
    out: dict[str, dict] = {}
    for f in sorted(glob.glob(str(data_dir() / "eval" / "finaltext" / "passages-*.jsonl"))):
        if any(tag in Path(f).name for tag in (".pre-", ".raw", ".v1", ".v2-only")):
            continue
        for line in open(f, encoding="utf-8"):
            p = json.loads(line)
            out.setdefault(p["id"], p)
    # Rows from passages later reworded or filtered out: an earlier snapshot holds their source,
    # URL and license (the same in every snapshot), newest first.
    for tag in (".pre-", ".v2-only", ".v1", ".raw"):
        for f in sorted(glob.glob(str(data_dir() / "eval" / "finaltext" / f"passages-*{tag}*.jsonl"))):
            for line in open(f, encoding="utf-8"):
                p = json.loads(line)
                out.setdefault(p["id"], p)
    return out


_LICENSES = [
    (r"by-sa/2\.5", "cc-by-sa-2.5"),
    (r"by-sa/3\.0", "cc-by-sa-3.0"),
    (r"by-sa/4\.0|^cc by-sa 4\.0$", "cc-by-sa-4.0"),
    (r"publicdomain/zero|^cc0", "cc0-1.0"),
    (r"^public domain$", "public-domain"),
    (r"^generated$", DATASET_LICENSE),
    (r"^mit$", "mit"),
    (r"^apache-2\.0$", "apache-2.0"),
    (r"^bsd-3-clause$", "bsd-3-clause"),
    (r"^bsd-2-clause$", "bsd-2-clause"),
    (r"^0bsd$", "0bsd"),
    (r"^upl-1\.0$", "upl-1.0"),
    (r"^bsl-1\.0$", "bsl-1.0"),
    (r"^mit-0$", "mit-0"),
    (r"^ecl-2\.0$", "ecl-2.0"),
    (r"^cc-by-4\.0$", "cc-by-4.0"),
    (r"^unlicense$", "unlicense"),
    (r"^isc$", "isc"),
    (r"^zlib$", "zlib"),
]


def license_id(raw: str) -> str:
    s = raw.strip().lower()
    for pattern, lid in _LICENSES:
        if re.search(pattern, s):
            return lid
    raise ValueError(f"unknown license {raw!r}")


def provenance(p: dict) -> dict:
    """source, source_url and license for a passage; synthetic ones point at what wrote them."""
    if p["source"] == "synthetic":
        url = SYNTHETIC_SCRIPT.get(p["domain"], SYNTHETIC_DEFAULT)
    else:
        url = p["url"]
    if not url:
        raise ValueError(f"passage {p['id']} has no URL")
    return {"source": p["source"], "source_url": url, "license": license_id(p["license"])}


# Mechanical punctuation slips in final texts, fixed only where there is no reading in which they
# are right: ".?" or "?." after a word (not after an abbreviation), ",." after a word, a space before
# a comma, semicolon or colon, and ".." that is not an ellipsis.
_ABBREVIATIONS = r"(?<!\betc)(?<!\be\.g)(?<!\bi\.e)(?<!\bvs)(?<!\bapprox)(?<!\bincl)(?<!\bMr)(?<!\bDr)(?<!\bSt)"
_FIXES = [
    (re.compile(r"(?<=\w)" + _ABBREVIATIONS + r"\.\?(?=\s|$)"), "?"),
    (re.compile(r"(?<=\w)\?\.(?=\s|$)"), "?"),
    (re.compile(r"(?<=\w),\.(?=\s|$)"), "."),
    (re.compile(r"(?<=\w) ([,;:])(?= )"), r"\1"),
    (re.compile(r"(?<=\w)(?<!\.)\.\.(?!\.)(?=\s|$)"), "."),
]


def fix_final(text: str) -> str:
    # Code spans and fenced blocks are left exactly as they are.
    parts = re.split(r"(```.*?```|`[^`\n]*`)", text, flags=re.S)
    for i in range(0, len(parts), 2):
        for pattern, repl in _FIXES:
            parts[i] = pattern.sub(repl, parts[i])
    return "".join(parts)


def write_parquet(
    path: Path, table: pa.Table, features: dict, row_group_size: int, binary_columns: list[str] = ()
) -> None:
    """One shard, with the features `datasets` reads from the schema metadata (key "huggingface").
    `binary_columns` (e.g. "audio.bytes", already compressed audio) are stored as they are: no
    compression, no dictionary, as the datasets library writes audio."""
    meta = dict(table.schema.metadata or {})
    meta[b"huggingface"] = json.dumps({"info": {"features": features}}).encode()
    table = table.replace_schema_metadata(meta)
    path.parent.mkdir(parents=True, exist_ok=True)
    leaves = _leaf_paths(table.schema)
    with pq.ParquetWriter(
        path,
        table.schema,
        compression={c: ("none" if c in binary_columns else "snappy") for c in leaves},
        use_dictionary=[c for c in leaves if c not in binary_columns],
        use_content_defined_chunking=True,
        write_page_index=True,
    ) as w:
        w.write_table(table, row_group_size=row_group_size)


def _leaf_paths(schema: pa.Schema) -> list[str]:
    """Parquet's dotted column paths ("audio.bytes", "annotations.list.element")."""
    out = []

    def walk(prefix: str, t: pa.DataType) -> None:
        if pa.types.is_struct(t):
            for f in t:
                walk(f"{prefix}.{f.name}", f.type)
        elif pa.types.is_list(t) or pa.types.is_large_list(t):
            walk(f"{prefix}.list.element", t.value_type)
        else:
            out.append(prefix)

    for f in schema:
        walk(f.name, f.type)
    return out


def shards(rows: list, max_bytes: int, size_of) -> list[list]:
    """Rows cut into shards of at most `max_bytes` (as `size_of` counts them), in order."""
    out: list[list] = [[]]
    total = 0
    for r in rows:
        s = size_of(r)
        if out[-1] and total + s > max_bytes:
            out.append([])
            total = 0
        out[-1].append(r)
        total += s
    return out
