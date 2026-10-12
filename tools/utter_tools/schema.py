"""The two published datasets' schemas, and the rules every part of the tooling follows with them.

dictation-to-text (config.TEXT_REPO): one row per dictation, what the recognizer heard and the
text that was meant, plus the exact chat the cleanup model trains on.
dictation-speech (config.SPEECH_REPO): one row per clip, the audio and what was spoken, and the
same final text; a corpus clip's `rendition_id` is its text row's `id`.

Ids: a passage has a 12-hex id; a rendition of it is `<passage>-<category>[-<n>]`; a speech row is
`<collection>/<rendition>`. A passage's renditions are always in the same split, in both datasets.
"""

from __future__ import annotations

import collections
import hashlib
import json
import re
from collections.abc import Callable, Iterable
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

DATASET_LICENSE = "cc-by-sa-4.0"
RATE = 16_000
# The share of rows in test (about 10,000 of 89,000).
TEST_SHARE = 0.112


def _value(dtype: str) -> dict:
    return {"dtype": dtype, "_type": "Value"}


_MESSAGE = pa.struct([("role", pa.string()), ("content", pa.string())])

TEXT_SCHEMA = pa.schema(
    [
        ("id", pa.string()),
        ("category", pa.string()),
        ("domain", pa.string()),
        ("dictated", pa.string()),
        ("final_text", pa.string()),
        ("dictionary", pa.list_(pa.string())),
        ("lists", pa.bool_()),
        ("messages", pa.list_(_MESSAGE)),
        ("source", pa.string()),
        ("source_url", pa.string()),
        ("license", pa.string()),
    ]
)
TEXT_FEATURES = {
    "id": _value("string"),
    "category": _value("string"),
    "domain": _value("string"),
    "dictated": _value("string"),
    "final_text": _value("string"),
    "dictionary": {"feature": _value("string"), "_type": "List"},
    "lists": _value("bool"),
    "messages": {
        "feature": {"role": _value("string"), "content": _value("string")},
        "_type": "List",
    },
    "source": _value("string"),
    "source_url": _value("string"),
    "license": _value("string"),
}

SPEECH_SCHEMA = pa.schema(
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
SPEECH_FEATURES = {
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
# The fields a passage carries into every row made from it.
PROVENANCE = ("source", "source_url", "license")
# How a passage is said (data.spoken): every category the datasets hold.
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
# Where passages come from: the corpora (data.passages, data.lists) and text Utter's tools wrote.
SOURCES = {
    "common-pile/stackexchange",
    "common-pile/arxiv_abstracts",
    "common-pile/ubuntu_irc",
    "wikimedia/wikipedia",
    "synthetic",
}
# The speech collections of the training corpus (each clip has its text row); the others are eval
# sets, all test.
CORPUS_PREFIX = "corpus"


def passage_id(row_id: str) -> str:
    """The passage a text row, rendition or speech row (`collection/rendition`) comes from."""
    return row_id.rsplit("/", 1)[-1].split("-")[0]


def new_passage_id(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:12]


def to_annotations(tags: dict[str, str]) -> list[str]:
    """A rendition's tags as the datasets store them: "key=value" strings."""
    return [f"{k}={v}" for k, v in tags.items()]


def from_annotations(annotations: Iterable[str]) -> dict[str, str]:
    return dict(a.split("=", 1) for a in annotations)


# Licenses: what the sources state, to the ids the datasets publish ("generated": text the tools
# wrote). Anything else is not published: license_id raises, and the passage is skipped.
PUBLISHED_LICENSES = {"cc-by-sa-4.0", "cc-by-sa-3.0", "cc-by-sa-2.5", "cc0-1.0", "public-domain"}
_LICENSES = [
    (r"by-sa/2\.5", "cc-by-sa-2.5"),
    (r"by-sa/3\.0", "cc-by-sa-3.0"),
    (r"by-sa/4\.0|^cc by-sa 4\.0$", "cc-by-sa-4.0"),
    (r"publicdomain/zero|^cc0", "cc0-1.0"),
    (r"^public domain$", "public-domain"),
    (r"^generated$", DATASET_LICENSE),
]


def license_id(raw: str) -> str:
    s = raw.strip().lower()
    for pattern, lid in _LICENSES:
        if re.search(pattern, s):
            return lid
    raise ValueError(f"unknown license {raw!r}")


# Mechanical punctuation slips in final texts, fixed only where there is no reading in which they
# are right: ".?" or "?." after a word (not after an abbreviation), ",." after a word, a space before
# a comma, semicolon or colon, and ".." that is not an ellipsis.
_ABBREVIATIONS = (
    r"(?<!\betc)(?<!\be\.g)(?<!\bi\.e)(?<!\bvs)(?<!\bapprox)(?<!\bincl)(?<!\bMr)(?<!\bDr)(?<!\bSt)"
)
_FIXES = [
    (re.compile(r"(?<=\w)" + _ABBREVIATIONS + r"\.\?(?=\s|$)"), "?"),
    (re.compile(r"(?<=\w)\?\.(?=\s|$)"), "?"),
    (re.compile(r"(?<=\w),\.(?=\s|$)"), "."),
    (re.compile(r"(?<=\w) ([,;:])(?= )"), r"\1"),
    (re.compile(r"(?<=\w)(?<!\.)\.\.(?!\.)(?=\s|$)"), "."),
]


def fix_final(text: str) -> str:
    # Code spans and fenced blocks are left exactly as they are.
    parts = re.split(r"(```.*?```|`[^`\n]*`)", text, flags=re.DOTALL)
    for i in range(0, len(parts), 2):
        for pattern, repl in _FIXES:
            parts[i] = pattern.sub(repl, parts[i])
    return "".join(parts)


def _length_tercile(n: int, cuts: tuple[int, int]) -> str:
    return "short" if n <= cuts[0] else "medium" if n <= cuts[1] else "long"


def split_by_passage(rows: list[dict], fixed: dict[str, str] | None = None) -> dict[str, str]:
    """Passage id -> "train" or "test" for text rows. A passage's renditions stay together (else
    test would hold near-copies of training rows). Passages in `fixed` keep their split (the
    published datasets' passages, when new ones are added). The others are grouped by everything
    that tells them apart (source, license, domain, the categories they are spoken in, length,
    whether any rendition has a dictionary or lists off), and each group gives TEST_SHARE of its
    rows to test, counting the fixed passages in it, in a fixed hash order, so every aspect stays
    balanced between the splits."""
    fixed = fixed or {}
    by_passage: dict[str, list[dict]] = collections.defaultdict(list)
    for r in rows:
        by_passage[passage_id(r["id"])].append(r)
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
            if pid in fixed:
                split[pid] = fixed[pid]
                taken += len(by_passage[pid]) * (fixed[pid] == "test")
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


def balance(rows: list[dict], split: dict[str, str]) -> dict[str, dict]:
    """Per aspect and value of text rows: the share of its rows in test."""
    lengths = sorted(len(r["final_text"]) for r in rows)
    cuts = (lengths[len(lengths) // 3], lengths[2 * len(lengths) // 3])
    aspects: dict[str, Callable[[dict], str]] = {
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
            test[v] += split[passage_id(r["id"])] == "test"
        out[name] = {v: round(test[v] / total[v], 4) for v in total if total[v]}
        out[name]["_rows"] = dict(total)
    return out


def write_parquet(
    path: Path,
    table: pa.Table,
    features: dict,
    row_group_size: int,
    binary_columns: Iterable[str] = (),
) -> None:
    """One shard, with the features `datasets` reads from the schema metadata (key "huggingface").
    `binary_columns` (e.g. "audio.bytes", already compressed audio) are stored as they are: no
    compression, no dictionary, as the datasets library writes audio."""
    binary = set(binary_columns)
    meta = dict(table.schema.metadata or {})
    meta[b"huggingface"] = json.dumps({"info": {"features": features}}).encode()
    table = table.replace_schema_metadata(meta)
    path.parent.mkdir(parents=True, exist_ok=True)
    leaves = _leaf_paths(table.schema)
    # Per-column compression and dictionary settings: pyarrow takes a dict and a list here, though
    # its type stubs declare only a str and a bool.
    with pq.ParquetWriter(
        path,
        table.schema,
        compression={c: ("none" if c in binary else "snappy") for c in leaves},  # pyright: ignore[reportArgumentType]
        use_dictionary=[c for c in leaves if c not in binary],  # pyright: ignore[reportArgumentType]
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


def shards(rows: list, max_bytes: int, size_of: Callable[[dict], int]) -> list[list]:
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
