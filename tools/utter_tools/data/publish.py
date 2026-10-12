"""Publishing a batch: its rows merged into the datasets, checked, then uploaded.

`build` writes `<batch>/publish/text/` (the whole text dataset: the Hub's rows plus the batch's,
split by passage, published passages keeping their split) unless the batch is an eval set, and
`<batch>/publish/speech/` (the batch's collection, FLAC audio, each clip in its text row's split),
each with its card (templates/*.md, every figure computed from the shards). `check` verifies what
the datasets promise, across the Hub's rows and the new ones; `upload` sends what was built.
"""

from __future__ import annotations

import collections
import io
import json
import re
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import soundfile as sf
from huggingface_hub import DatasetCard, HfApi, HfFileSystem

from utter_tools import common, schema
from utter_tools.data.batch import Batch

TEMPLATES = Path(__file__).resolve().parents[1] / "templates"
CORPORA = [
    "common-pile/stackexchange",
    "common-pile/arxiv_abstracts",
    "common-pile/ubuntu_irc",
    "wikimedia/wikipedia",
]
# What the speech card needs of each shard, read without the audio.
SUMMARY_COLUMNS = ["duration_s", "license", "source", "category", "tts_engine", "tts_settings"]
# Kyutai's donated voices (envs/tts/speak.py): their file names hold the donors' names.
VOICE_REPO = "kyutai/tts-voices"
VOICE_REVISION = "323332d33f997de8394f24a193e1a76df720e01a"


def folder(batch: Batch) -> Path:
    return batch.root / "publish"


def build_info(batch: Batch) -> dict:
    path = folder(batch) / "build.json"
    if not path.is_file():
        raise FileNotFoundError(f"batch {batch.meta.name} is not built: utter-data build")
    return json.loads(path.read_text())


# ---------------------------------------------------------------------------------------------
# Cards.


def _table(rows: list[list], header: list[str]) -> str:
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(lines)


def _count_table(
    counts: collections.Counter, header: list[str], meanings: dict | None = None
) -> str:
    rows = []
    for key, n in counts.most_common():
        rows.append([key, meanings.get(key, ""), f"{n:,}"] if meanings else [key, f"{n:,}"])
    return _table(rows, header)


def _features_yaml(s: pa.Schema) -> str:
    """The card's dataset_info features, from the schema."""
    out = []
    for f in s:
        if f.name == "messages":
            out.append(
                f"  - name: {f.name}\n    list:\n    - name: role\n      dtype: string\n"
                "    - name: content\n      dtype: string"
            )
        elif f.name == "audio":
            out.append(
                f"  - name: {f.name}\n    dtype:\n      audio:\n"
                f"        sampling_rate: {schema.RATE}"
            )
        elif pa.types.is_list(f.type):
            out.append(f"  - name: {f.name}\n    list: string")
        else:
            out.append(f"  - name: {f.name}\n    dtype: {schema.features(s)[f.name]['dtype']}")
    return "\n".join(out)


def text_card(out: Path) -> None:
    files = sorted((out / "data").glob("*.parquet"))
    columns = ["id", "category", "domain", "source", "license"]
    rows = [
        {**r, "_split": f.name.split("-")[0]}
        for f in files
        for r in pq.read_table(f, columns=columns).to_pylist()
    ]
    # Test's share of every value with 500 rows or more, of every aspect the split balances.
    shares = []
    for aspect in ("category", "domain", "source", "license"):
        totals = collections.Counter(r[aspect] for r in rows)
        tests = collections.Counter(r[aspect] for r in rows if r["_split"] == "test")
        shares += [tests[value] / n for value, n in totals.items() if n >= 500]
    counts = collections.Counter(r["_split"] for r in rows)
    template = (TEMPLATES / "text-card.md").read_text(encoding="utf-8")
    card = template.format(
        source_datasets="\n".join(f"- {s}" for s in CORPORA),
        features=_features_yaml(schema.TEXT),
        train_bytes=sum(f.stat().st_size for f in files if f.name.startswith("train")),
        test_bytes=sum(f.stat().st_size for f in files if f.name.startswith("test")),
        train_rows=counts["train"],
        test_rows=counts["test"],
        total=len(rows),
        balance_lo=100 * min(shares),
        balance_hi=100 * max(shares),
        categories=_count_table(
            collections.Counter(r["category"] for r in rows),
            ["category", "spoken", "rows"],
            schema.CATEGORIES,
        ),
        sources=_count_table(collections.Counter(r["source"] for r in rows), ["source", "rows"]),
        licenses=_count_table(collections.Counter(r["license"] for r in rows), ["license", "rows"]),
    )
    (out / "README.md").write_text(card, encoding="utf-8", newline="\n")


def summarize(table: pa.Table, split: str, collection: str, nbytes: int) -> dict:
    """What the speech card needs of one shard (`table` holds SUMMARY_COLUMNS)."""
    settings = table.column("tts_settings").to_pylist()
    return {
        "split": split,
        "collection": collection,
        "rows": table.num_rows,
        "bytes": nbytes,
        "hours": sum(table.column("duration_s").to_pylist()) / 3600,
        "regenerated": sum("audio=regenerated" in s for s in settings),
        **{
            c: dict(collections.Counter(table.column(c).to_pylist()))
            for c in ("license", "source", "category", "tts_engine")
        },
    }


def hub_summaries(rev: str) -> list[dict]:
    """Summaries of the speech dataset's shards on the Hub, reading only their small columns."""
    fs = HfFileSystem()
    out = []
    for path in sorted(fs.glob(f"datasets/{common.SPEECH_REPO}@{rev}/data/*/*.parquet")):
        collection, name = path.split("/")[-2:]
        table = pq.read_table(path, columns=SUMMARY_COLUMNS, filesystem=fs)
        out.append(summarize(table, name.split("-")[0], collection, int(fs.size(path) or 0)))
    return out


def speech_card(out: Path, shards: list[dict]) -> None:
    """The card from the summaries of every shard: the Hub's and the new ones."""
    names = sorted(
        {s["collection"] for s in shards}, key=lambda c: (not c.startswith(schema.CORPUS), c)
    )
    rows, hours, size = collections.Counter(), collections.Counter(), collections.Counter()
    totals = {c: collections.Counter() for c in ("license", "source", "category", "tts_engine")}
    for s in shards:
        rows[s["split"]] += s["rows"]
        hours[s["split"]] += s["hours"]
        size[s["split"]] += s["bytes"]
        for column, counter in totals.items():
            counter.update(s[column])
    splits = [s for s in ("train", "test") if rows[s]]

    configs = ["- config_name: all\n  default: true\n  data_files:"]
    configs += [f"  - split: {s}\n    path: data/*/{s}-*.parquet" for s in splits]
    collection_rows = []
    for c in names:
        mine = [s for s in shards if s["collection"] == c]
        per_split = collections.Counter()
        for s in mine:
            per_split[s["split"]] += s["rows"]
        configs.append(f"- config_name: {c}\n  data_files:")
        configs += [
            f"  - split: {s}\n    path: data/{c}/{s}-*.parquet"
            for s in ("train", "test")
            if per_split[s]
        ]
        collection_rows.append(
            [
                c,
                f"{per_split['train']:,}" if per_split["train"] else "",
                f"{per_split['test']:,}" if per_split["test"] else "",
                f"{sum(s['hours'] for s in mine):.1f}",
                ", ".join(sorted({k for s in mine for k in s["category"]})),
            ]
        )
    template = (TEMPLATES / "speech-card.md").read_text(encoding="utf-8")
    card = template.format(
        size_category="100K<n<1M" if sum(rows.values()) >= 100_000 else "10K<n<100K",
        source_datasets="\n".join(f"- {s}" for s in CORPORA),
        configs="\n".join(configs),
        features=_features_yaml(schema.SPEECH),
        splits="\n".join(
            f"  - name: {s}\n    num_bytes: {size[s]}\n    num_examples: {rows[s]}" for s in splits
        ),
        total=sum(rows.values()),
        hours=sum(hours.values()),
        split_table="\n".join(f"| {s} | {rows[s]:,} | {hours[s]:.0f} |" for s in splits),
        collections=_table(collection_rows, ["collection", "train", "test", "hours", "categories"]),
        engines=", ".join(f"`{e}` ({n:,})" for e, n in totals["tts_engine"].most_common()),
        categories=_count_table(
            totals["category"], ["category", "spoken", "clips"], schema.CATEGORIES
        ),
        sources=_count_table(totals["source"], ["source", "clips"]),
        licenses=_count_table(totals["license"], ["license", "clips"]),
        regenerated=sum(s["regenerated"] for s in shards),
    )
    (out / "README.md").write_text(card, encoding="utf-8", newline="\n")


# ---------------------------------------------------------------------------------------------
# Building.


def _text_size(row: dict) -> int:
    return 2 * sum(len(m["content"]) for m in row["messages"])


def _clip_size(row: dict) -> int:
    return len(row["audio"]["bytes"])


def _flac(clip: dict, clips: Path) -> dict:
    """A clip's manifest row as a speech row with its audio, FLAC (lossless)."""
    audio, rate = sf.read(clips / clip["audio"], dtype="int16")
    buffer = io.BytesIO()
    sf.write(buffer, audio, rate, format="FLAC", subtype="PCM_16")
    return {
        **clip,
        "audio": {"bytes": buffer.getvalue(), "path": f"{clip['rendition_id']}.flac"},
        "duration_s": round(len(audio) / rate, 2),
    }


def build(batch: Batch) -> dict:
    out = folder(batch)
    text_rev = common.revision(common.TEXT_REPO)
    speech_rev = common.revision(common.SPEECH_REPO)
    new_text = common.read_jsonl(batch.text)
    if batch.meta.test_only:
        split = {schema.passage_id(r["id"]): "test" for r in common.read_jsonl(batch.renditions)}
    else:
        published = {s: common.text_rows(text_rev, s) for s in ("train", "test")}
        fixed = {schema.passage_id(r["id"]): s for s, rows in published.items() for r in rows}
        rows = published["train"] + published["test"] + new_text
        split = schema.split_by_passage(rows, fixed)
        for part in ("train", "test"):
            mine = [r for r in rows if split[schema.passage_id(r["id"])] == part]
            schema.write_shards(
                mine, out / "text" / "data", part, schema.TEXT, 300 << 20, 10_000, _text_size
            )
        text_card(out / "text")

    # A corpus clip goes with its text row (one clip per row); an eval batch keeps every clip.
    paired = {r["id"] for r in new_text}
    clips = [
        _flac(c, batch.clips)
        for c in common.read_jsonl(batch.clips / "manifest.jsonl")
        if batch.meta.test_only or c["rendition_id"] in paired
    ]
    speech = out / "speech"
    for part in ("train", "test"):
        mine = [c for c in clips if split[schema.passage_id(c["rendition_id"])] == part]
        if mine:
            target = speech / "data" / batch.meta.collection
            schema.write_shards(mine, target, part, schema.SPEECH, 500 << 20, 100, _clip_size)
    local = [
        summarize(
            pq.read_table(f, columns=SUMMARY_COLUMNS),
            f.name.split("-")[0],
            f.parent.name,
            f.stat().st_size,
        )
        for f in sorted((speech / "data").glob("*/*.parquet"))
    ]
    speech_card(speech, hub_summaries(speech_rev) + local)
    info = {common.TEXT_REPO: text_rev, common.SPEECH_REPO: speech_rev}
    info |= {"text_rows": len(new_text), "clips": len(clips)}
    (out / "build.json").write_text(json.dumps(info, indent=2))
    return info


# ---------------------------------------------------------------------------------------------
# Checking.


class Checks:
    """Prints each check; the failed ones block the upload."""

    def __init__(self) -> None:
        self.failed: list[str] = []

    def __call__(self, ok: bool, what: str, detail: str = "") -> None:
        print(f"  [{'ok' if ok else 'FAIL'}] {what}" + (f": {detail}" if detail else ""))
        if not ok:
            self.failed.append(what)


def _shard_rows(root: Path, columns: list[str] | None = None) -> list[dict]:
    return [
        {**r, "_split": f.name.split("-")[0]}
        for f in sorted(root.glob("data/**/*.parquet"))
        for r in pq.read_table(f, columns=columns).to_pylist()
    ]


def _card(check: Checks, card: Path, rows: list[dict]) -> None:
    c = DatasetCard.load(card)
    try:
        c.validate(repo_type="dataset")
        check(True, "card YAML validates")
    except Exception as e:  # noqa: BLE001 -- any validation error is a failed check
        check(False, "card YAML validates", str(e)[:200])
    declared = {s["name"]: s["num_examples"] for s in c.data.to_dict()["dataset_info"]["splits"]}
    actual = dict(collections.Counter(r["_split"] for r in rows))
    check(declared == actual, "card split counts match the shards", str(declared))


def _one_split(check: Checks, rows: list[dict], id_field: str) -> None:
    splits = collections.defaultdict(set)
    for r in rows:
        splits[schema.passage_id(r[id_field])].add(r["_split"])
    both = [p for p, s in splits.items() if len(s) > 1]
    check(not both, "no passage in both splits", str(len(both)))


def _chat_agrees(row: dict) -> bool:
    """The training chat says what the row's fields say, as the app would send it."""
    system, user, assistant = row["messages"]
    dictionary = re.search(r"^Dictionary: (.*)$", user["content"], re.MULTILINE)
    terms = [t.strip() for t in dictionary.group(1).split(",")] if dictionary else []
    return (
        [system["role"], user["role"], assistant["role"]] == ["system", "user", "assistant"]
        and assistant["content"] == row["final_text"]
        and user["content"].endswith("Dictated: " + row["dictated"])
        and terms == row["dictionary"]
        and user["content"].startswith("Lists: off\n") != row["lists"]
    )


def check_text(check: Checks, out: Path) -> list[dict]:
    rows = _shard_rows(out)
    check(len({r["id"] for r in rows}) == len(rows), "ids unique", f"{len(rows)} rows")
    check({r["category"] for r in rows} <= set(schema.CATEGORIES), "categories known")
    check({r["license"] for r in rows} <= schema.LICENSES, "licenses are published ones")
    check({r["source"] for r in rows} <= schema.SOURCES, "sources known")
    check(all(r["source_url"].startswith("https://") for r in rows), "source URLs are https")
    check(len({r["messages"][0]["content"] for r in rows}) == 1, "one system prompt")
    disagree = [r["id"] for r in rows if not _chat_agrees(r)]
    check(not disagree, "chat agrees with the row", f"{len(disagree)} {disagree[:2]}")
    check(
        all(schema.fix_final(r["final_text"]) == r["final_text"] for r in rows),
        "no mechanical slips",
    )
    _one_split(check, rows, "id")
    _card(check, out / "README.md", rows)
    return rows


def _decodes(row: dict) -> bool:
    info = sf.info(io.BytesIO(row["audio"]["bytes"]))
    return (
        info.format == "FLAC"
        and info.samplerate == schema.RATE
        and info.channels == 1
        and abs(info.frames / schema.RATE - row["duration_s"]) <= 0.011
    )


def check_speech(check: Checks, batch: Batch, text: list[dict], rev: str) -> None:
    out = folder(batch) / "speech"
    new = _shard_rows(out)
    check(all(_decodes(r) for r in new), "every clip is FLAC, 16 kHz, mono, as long as duration_s")
    check(
        all(r["id"] == f"{r['collection']}/{r['rendition_id']}" for r in new),
        "id = collection/rendition_id",
    )
    check({r["license"] for r in new} <= schema.LICENSES, "licenses are published ones")
    files = HfApi().list_repo_files(VOICE_REPO, revision=VOICE_REVISION)
    donors = {
        Path(f).stem.removesuffix("_enhanced") for f in files if f.startswith("voice-donations/")
    }
    generated = json.dumps(
        [[r["tts_voice"], r["tts_settings"], r["annotations"], r["audio"]["path"]] for r in new]
    )
    named = [d for d in donors if re.search(rf"\b{re.escape(d)}\b", generated, re.IGNORECASE)]
    check(not named, "no donor's name in the speech fields", str(named))

    # Across the whole dataset: the Hub's rows and the new ones.
    small = ["id", "rendition_id", "collection", "final_text"]
    everything = [{**r, "_split": r.pop("split")} for r in common.speech_columns(rev, small)]
    everything += [{k: r[k] for k in [*small, "_split"]} for r in new]
    check(len({r["id"] for r in everything}) == len(everything), "ids unique across the dataset")
    _one_split(check, everything, "rendition_id")
    pairs = {t["id"]: t for t in text}
    corpus = [r for r in everything if r["collection"].startswith(schema.CORPUS)]
    per_row = collections.Counter(r["rendition_id"] for r in corpus)
    check(all(per_row[i] == 1 for i in pairs), "every text row has exactly one corpus clip")
    check(all(r["rendition_id"] in pairs for r in corpus), "every corpus clip has its text row")
    wrong = []
    for r in corpus:
        pair = pairs.get(r["rendition_id"])
        if pair and (
            pair["_split"] != r["_split"] or pair["lists"] and pair["final_text"] != r["final_text"]
        ):
            wrong.append(r["id"])
    check(not wrong, "each clip in its text row's split, with its final text", str(len(wrong)))
    _card(check, out / "README.md", everything)


def check(batch: Batch) -> list[str]:
    """Every check; returns the failed ones."""
    info = build_info(batch)
    checks = Checks()
    if batch.meta.test_only:
        rev = info[common.TEXT_REPO]
        columns = ["id", "final_text", "lists"]
        text = [
            {**r, "_split": s} for s in ("train", "test") for r in common.text_rows(rev, s, columns)
        ]
    else:
        print("dictation-to-text")
        text = check_text(checks, folder(batch) / "text")
    print("dictation-speech")
    check_speech(checks, batch, text, info[common.SPEECH_REPO])
    return checks.failed


def upload(batch: Batch) -> None:
    out = folder(batch)
    info = build_info(batch)
    for repo in (common.TEXT_REPO, common.SPEECH_REPO):
        if common.revision(repo) != info[repo]:
            raise RuntimeError(f"{repo} changed since the build: build again")
    api = HfApi()
    if not batch.meta.test_only:
        api.upload_folder(
            repo_id=common.TEXT_REPO,
            repo_type="dataset",
            folder_path=out / "text",
            allow_patterns=["README.md", "data/*.parquet"],
            delete_patterns=["data/*.parquet"],
            commit_message=f"Add batch {batch.meta.name}",
        )
    api.upload_folder(
        repo_id=common.SPEECH_REPO,
        repo_type="dataset",
        folder_path=out / "speech",
        allow_patterns=["README.md", f"data/{batch.meta.collection}/*.parquet"],
        commit_message=f"Add collection {batch.meta.collection}",
    )
