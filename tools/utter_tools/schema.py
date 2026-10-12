"""The two datasets' schemas and the rules every command follows with them.

dictation-to-text: a dictation as the recognizer heard it, the text that was meant, and the exact
chat the cleanup model trains on. dictation-speech: a clip, what was said, and the same final
text; a corpus clip's `rendition_id` is its text row's `id`; the eval collections are speech only,
all in test.

Ids: a passage is 12 hex digits, a rendition is `<passage>-<category>`, a speech row is
`<collection>/<rendition>`. A passage's rows are always in one split, in both datasets.
"""

from __future__ import annotations

import collections
import hashlib
import json
import re
from collections.abc import Callable
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

RATE = 16_000
# The share of rows in test (about 10,000 of 89,000).
TEST_SHARE = 0.112
DATASET_LICENSE = "cc-by-sa-4.0"
# The only licenses the datasets publish.
LICENSES = {"cc-by-sa-4.0", "cc-by-sa-3.0", "cc-by-sa-2.5", "cc0-1.0", "public-domain"}
# Speech collections paired with text rows; the others are eval sets.
CORPUS = "corpus"
CATEGORIES = {
    "natural": "the passage said plainly",
    "disfluent": "with fillers, repeats and false starts",
    "correction": 'with a spoken self-correction ("three, no, four")',
    "command": 'with a spoken editing command ("scratch that")',
    "explicit": 'with spoken punctuation ("comma", "new paragraph")',
    "amend": "with a later edit to something said earlier",
    "list": "a list, spoken item by item",
    "grammar": "with the grammar slips of real speakers, to be corrected",
    "restate": "said again in other words partway through",
}
SOURCES = {
    "common-pile/stackexchange",
    "common-pile/arxiv_abstracts",
    "common-pile/ubuntu_irc",
    "wikimedia/wikipedia",
    "synthetic",
}
# The fields a passage gives every row made from it.
PROVENANCE = ("source", "source_url", "license")

_STRING = pa.string()
_STRINGS = pa.list_(pa.string())
TEXT = pa.schema(
    [
        ("id", _STRING),
        ("category", _STRING),
        ("domain", _STRING),
        ("dictated", _STRING),
        ("final_text", _STRING),
        ("dictionary", _STRINGS),
        ("lists", pa.bool_()),
        ("messages", pa.list_(pa.struct([("role", _STRING), ("content", _STRING)]))),
        ("source", _STRING),
        ("source_url", _STRING),
        ("license", _STRING),
    ]
)
SPEECH = pa.schema(
    [
        ("id", _STRING),
        ("rendition_id", _STRING),
        ("audio", pa.struct([("bytes", pa.binary()), ("path", _STRING)])),
        ("duration_s", pa.float32()),
        ("language", _STRING),
        ("transcript", _STRING),
        ("final_text", _STRING),
        ("category", _STRING),
        ("domain", _STRING),
        ("annotations", _STRINGS),
        ("tts_engine", _STRING),
        ("tts_voice", _STRING),
        ("tts_settings", _STRINGS),
        ("collection", _STRING),
        ("source", _STRING),
        ("source_url", _STRING),
        ("license", _STRING),
    ]
)


def _feature(t: pa.DataType) -> dict:
    """The `datasets` library's feature for an Arrow type; the audio struct is Audio."""
    if pa.types.is_list(t):
        return {"feature": _feature(t.value_type), "_type": "List"}
    if pa.types.is_struct(t):
        if [f.name for f in t] == ["bytes", "path"]:
            return {"sampling_rate": RATE, "_type": "Audio"}
        return {f.name: _feature(f.type) for f in t}
    if pa.types.is_boolean(t):
        dtype = "bool"
    elif pa.types.is_floating(t):
        dtype = f"float{t.bit_width}"
    else:
        dtype = str(t)
    return {"dtype": dtype, "_type": "Value"}


def features(schema: pa.Schema) -> dict:
    return {f.name: _feature(f.type) for f in schema}


def passage_id(row_id: str) -> str:
    """The passage of a text row, a rendition or a speech row."""
    return row_id.rsplit("/", 1)[-1].split("-")[0]


def new_passage_id(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:12]


def tags(annotations: list[str]) -> dict[str, str]:
    """A row's annotations ("key=value") as a dict."""
    return dict(a.split("=", 1) for a in annotations)


_LICENSE_PATTERNS = [
    (r"by-sa/2\.5", "cc-by-sa-2.5"),
    (r"by-sa/3\.0", "cc-by-sa-3.0"),
    (r"by-sa/4\.0|^cc by-sa 4\.0$", "cc-by-sa-4.0"),
    (r"publicdomain/zero|^cc0", "cc0-1.0"),
    (r"^public domain$", "public-domain"),
    (r"^generated$", DATASET_LICENSE),
]


def license_id(raw: str) -> str | None:
    """The dataset's id for the license a source states, or None for one it does not publish."""
    s = raw.strip().lower()
    for pattern, lid in _LICENSE_PATTERNS:
        if re.search(pattern, s):
            return lid
    return None


# Mechanical slips in final texts, fixed only where no reading makes them right: ".?" or "?."
# after a word (not an abbreviation), ",." after a word, a space before a comma, semicolon or
# colon, and ".." that is not an ellipsis.
_NOT_ABBREVIATION = (
    r"(?<!\betc)(?<!\be\.g)(?<!\bi\.e)(?<!\bvs)(?<!\bapprox)(?<!\bincl)(?<!\bMr)(?<!\bDr)(?<!\bSt)"
)
_FIXES = [
    (re.compile(r"(?<=\w)" + _NOT_ABBREVIATION + r"\.\?(?=\s|$)"), "?"),
    (re.compile(r"(?<=\w)\?\.(?=\s|$)"), "?"),
    (re.compile(r"(?<=\w),\.(?=\s|$)"), "."),
    (re.compile(r"(?<=\w) ([,;:])(?= )"), r"\1"),
    (re.compile(r"(?<=\w)(?<!\.)\.\.(?!\.)(?=\s|$)"), "."),
]


def fix_final(text: str) -> str:
    """The final text with mechanical slips fixed; code spans are left exactly as they are."""
    parts = re.split(r"(```.*?```|`[^`\n]*`)", text, flags=re.DOTALL)
    for i in range(0, len(parts), 2):
        for pattern, replacement in _FIXES:
            parts[i] = pattern.sub(replacement, parts[i])
    return "".join(parts)


def split_by_passage(rows: list[dict], fixed: dict[str, str] | None = None) -> dict[str, str]:
    """Passage id -> "train" or "test", for text rows.

    Passages in `fixed` keep their split (those already published). The rest are grouped by
    everything that tells them apart (source, license, domain, categories, length, whether any
    rendition has a dictionary or lists off), and each group gives TEST_SHARE of its rows to test,
    in a fixed hash order, so every aspect stays balanced between the splits.
    """
    fixed = fixed or {}
    by_passage: dict[str, list[dict]] = collections.defaultdict(list)
    for r in rows:
        by_passage[passage_id(r["id"])].append(r)
    lengths = sorted(len(rs[0]["final_text"]) for rs in by_passage.values())
    short, medium = lengths[len(lengths) // 3], lengths[2 * len(lengths) // 3]

    groups: dict[tuple, list[str]] = collections.defaultdict(list)
    for pid, rs in by_passage.items():
        length = len(rs[0]["final_text"])
        key = (
            rs[0]["source"],
            rs[0]["license"],
            rs[0]["domain"],
            tuple(sorted({r["category"] for r in rs})),
            0 if length <= short else 1 if length <= medium else 2,
            any(r["dictionary"] for r in rs),
            any(not r["lists"] for r in rs),
        )
        groups[key].append(pid)

    split = dict(fixed)
    carry = 0.0  # rows owed to test by groups too small to give a whole passage
    for key in sorted(groups):
        pids = sorted(groups[key], key=lambda p: hashlib.sha256(p.encode()).hexdigest())
        want = TEST_SHARE * sum(len(by_passage[p]) for p in pids) + carry
        taken = sum(len(by_passage[p]) for p in pids if fixed.get(p) == "test")
        for pid in pids:
            if pid in fixed:
                continue
            n = len(by_passage[pid])
            if taken + n / 2 <= want:
                split[pid] = "test"
                taken += n
            else:
                split[pid] = "train"
        carry = want - taken
    return split


def _leaf_columns(schema: pa.Schema) -> list[str]:
    """Parquet's dotted column paths ("audio.bytes", "annotations.list.element")."""

    def walk(prefix: str, t: pa.DataType) -> list[str]:
        if pa.types.is_struct(t):
            return [p for f in t for p in walk(f"{prefix}.{f.name}", f.type)]
        if pa.types.is_list(t):
            return walk(f"{prefix}.list.element", t.value_type)
        return [prefix]

    return [p for f in schema for p in walk(f.name, f.type)]


def write_shards(
    rows: list[dict],
    folder: Path,
    split: str,
    schema: pa.Schema,
    shard_bytes: int,
    row_group: int,
    size_of: Callable[[dict], int],
) -> None:
    """Rows as Hub Parquet shards (`<split>-00000-of-0000N.parquet`) of about `shard_bytes`, with
    the features `datasets` reads from the schema metadata. Audio (already FLAC) is stored as it
    is: no compression and no dictionary, as the datasets library writes it."""
    shards: list[list[dict]] = [[]]
    size = 0
    for r in rows:
        if shards[-1] and size + size_of(r) > shard_bytes:
            shards.append([])
            size = 0
        shards[-1].append(r)
        size += size_of(r)

    columns = _leaf_columns(schema)
    metadata = {b"huggingface": json.dumps({"info": {"features": features(schema)}}).encode()}
    schema = schema.with_metadata(metadata)
    folder.mkdir(parents=True, exist_ok=True)
    for i, shard in enumerate(shards):
        path = folder / f"{split}-{i:05d}-of-{len(shards):05d}.parquet"
        # Per-column settings: pyarrow takes a dict and a list here, though its stubs declare a
        # str and a bool.
        compression = {c: "none" if c == "audio.bytes" else "snappy" for c in columns}
        dictionary = [c for c in columns if c != "audio.bytes"]
        with pq.ParquetWriter(
            path,
            schema,
            compression=compression,  # pyright: ignore[reportArgumentType]
            use_dictionary=dictionary,  # pyright: ignore[reportArgumentType]
            use_content_defined_chunking=True,
            write_page_index=True,
        ) as writer:
            writer.write_table(pa.Table.from_pylist(shard, schema=schema), row_group_size=row_group)
