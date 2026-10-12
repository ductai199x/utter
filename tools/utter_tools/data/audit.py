"""Audits a batch's build (data.publish) before upload: every row and every file, against the rules
the datasets promise. Each check is printed; any failure blocks the upload.

The text dataset is checked whole (the build holds all of it). The speech dataset's new shards are
checked clip by clip (every clip decoded, a sample compared with its WAV); ids, splits and the
pairing with the text rows are checked across every shard, the Hub's (their small columns, read
remotely) and the new ones.
"""

from __future__ import annotations

import collections
import glob
import io
import json
import random
import re
from pathlib import Path

import pyarrow.parquet as pq
import soundfile as sf
from huggingface_hub import HfApi

from utter_tools import config, hub, schema

MAX_ROW_GROUP = 300e6

# Kyutai's donated voices (envs/tts/speak.py): their file names hold the donors' names.
VOICE_REPO = "kyutai/tts-voices"
VOICE_REVISION = "323332d33f997de8394f24a193e1a76df720e01a"


class Audit:
    def __init__(self) -> None:
        self.failures: list[str] = []

    def check(self, ok: bool, what: str, detail: str = "") -> None:
        print(f"  [{'ok' if ok else 'FAIL'}] {what}{(': ' + detail) if detail else ''}")
        if not ok:
            self.failures.append(what)


def _files(out: Path) -> list[str]:
    return sorted(glob.glob(str(out / "data" / "**" / "*.parquet"), recursive=True))


def _file_checks(a: Audit, files: list[str], expect_features: set[str]) -> None:
    print("files")
    for f in files:
        pf = pq.ParquetFile(f)
        meta = json.loads(pf.schema_arrow.metadata[b"huggingface"])["info"]["features"]
        md = pf.metadata
        rg = max(md.row_group(i).total_byte_size for i in range(md.num_row_groups))
        name = Path(f).name
        a.check(
            set(meta) == expect_features == set(pf.schema_arrow.names),
            f"{name}: features declared",
            f"{len(meta)} columns",
        )
        a.check(
            rg <= MAX_ROW_GROUP, f"{name}: row groups at most 300 MB", f"largest {rg / 1e6:.0f} MB"
        )
        a.check(
            re.fullmatch(r"(train|test)-\d{5}-of-\d{5}\.parquet", name) is not None,
            f"{name}: Hub file name",
        )
    print()


def _card_checks(a: Audit, out: Path, expected: dict[str, int]) -> None:
    from huggingface_hub import DatasetCard

    print("card")
    card = out / "README.md"
    if not card.is_file():
        print("  [info] no README.md yet: the card is checked once it is written")
        return
    c = DatasetCard.load(card)
    try:
        c.validate(repo_type="dataset")
        a.check(True, "YAML validates against the Hub's schema")
    except Exception as e:  # noqa: BLE001 -- any validation error is a failed check
        a.check(False, "YAML validates against the Hub's schema", str(e)[:200])
    splits = {s["name"]: s["num_examples"] for s in c.data.to_dict()["dataset_info"]["splits"]}
    a.check(splits == expected, "card split counts match the files", f"{splits}")
    print()


def _no_empty(a: Audit, rows: list[dict], columns: list[str], where: str) -> None:
    for col in columns:
        bad = [r["id"] for r in rows if r[col] is None or r[col] == ""]
        a.check(not bad, f"{where}: no empty {col}", f"{len(bad)} e.g. {bad[:2]}" if bad else "")


def text(out: Path) -> list[str]:
    """Audits a built text dataset folder; returns the failed checks."""
    a = Audit()
    files = _files(out)
    _file_checks(
        a,
        files,
        {
            "id",
            "category",
            "domain",
            "dictated",
            "final_text",
            "dictionary",
            "lists",
            "messages",
            "source",
            "source_url",
            "license",
        },
    )
    split_rows = {
        s: [r for f in files if Path(f).name.startswith(s) for r in pq.read_table(f).to_pylist()]
        for s in ("train", "test")
    }
    allrows = split_rows["train"] + split_rows["test"]
    print("rows")
    _no_empty(
        a,
        allrows,
        ["id", "category", "domain", "dictated", "final_text", "source", "source_url", "license"],
        "all",
    )
    a.check(
        all(
            r["dictionary"] is not None and r["lists"] is not None and r["messages"]
            for r in allrows
        ),
        "list and bool columns never null",
    )
    a.check(
        len({r["id"] for r in allrows}) == len(allrows),
        "ids unique across splits",
        f"{len(allrows)} rows",
    )
    a.check(
        {r["category"] for r in allrows} <= set(schema.CATEGORIES),
        "categories known",
        str(sorted({r["category"] for r in allrows})),
    )
    a.check(
        {r["license"] for r in allrows} <= schema.PUBLISHED_LICENSES,
        "licenses known",
        f"{len({r['license'] for r in allrows})} ids",
    )
    a.check({r["source"] for r in allrows} <= schema.SOURCES, "sources known")
    a.check(all(r["source_url"].startswith("https://") for r in allrows), "source URLs are https")
    a.check(
        all(r["id"].endswith("-" + r["category"]) for r in allrows), "id = passage id + category"
    )
    bad = [
        r["id"]
        for r in allrows
        if [m["role"] for m in r["messages"]] != ["system", "user", "assistant"]
    ]
    a.check(not bad, "messages are system, user, assistant", f"{bad[:2]}")
    bad = [r["id"] for r in allrows if r["messages"][2]["content"] != r["final_text"]]
    a.check(not bad, "the assistant message is final_text", f"{len(bad)}")
    bad = []
    for r in allrows:
        user = r["messages"][1]["content"]
        header = user[: user.index("Dictated: ")]
        dic = re.search(r"Dictionary: ([^\n]*)", header)
        terms = [t.strip() for t in dic.group(1).split(",") if t.strip()] if dic else []
        if (
            not user.endswith("Dictated: " + r["dictated"])
            or terms != r["dictionary"]
            or ("Lists: off" not in header) != r["lists"]
        ):
            bad.append(r["id"])
    a.check(
        not bad,
        "dictated, dictionary and lists agree with the user message",
        f"{len(bad)} e.g. {bad[:2]}",
    )
    a.check(len({r["messages"][0]["content"] for r in allrows}) == 1, "one system prompt")
    bad = [r["id"] for r in allrows if schema.fix_final(r["final_text"]) != r["final_text"]]
    a.check(not bad, "no fixable punctuation slip left in final_text", f"{len(bad)}")
    trp = {schema.passage_id(r["id"]) for r in split_rows["train"]}
    leak = [r["id"] for r in split_rows["test"] if schema.passage_id(r["id"]) in trp]
    a.check(not leak, "no test passage in train", f"{len(leak)}")
    dup = collections.Counter((r["dictated"], r["final_text"]) for r in allrows)
    n = sum(c - 1 for c in dup.values() if c > 1)
    print(f"  [info] exact duplicate (dictated, final_text) pairs: {n}")
    print()
    _card_checks(a, out, {s: len(v) for s, v in split_rows.items()})
    return a.failures


def _donors() -> set[str]:
    files = HfApi().list_repo_files(VOICE_REPO, revision=VOICE_REVISION)
    return {
        Path(f).stem.removesuffix("_enhanced")
        for f in files
        if f.startswith("voice-donations/") and f.endswith("_enhanced.wav")
    }


def text_rows(text_dir: Path) -> list[dict]:
    """A text dataset's rows (the columns the speech audit pairs on), with their split."""
    return [
        {**t, "_split": Path(f).name.split("-")[0]}
        for f in _files(text_dir)
        for t in pq.read_table(f, columns=["id", "final_text", "lists"]).to_pylist()
    ]


def speech(out: Path, text: list[dict], clips: Path, rev: str, sample: int = 300) -> list[str]:
    """Audits a batch's speech build (`out`) against the text dataset it pairs with (`text`, rows
    with their split: the batch's text build, or the Hub's for an eval batch), its clips (the
    WAVs they were made from) and the Hub's speech dataset at `rev`; returns the failed checks."""
    a = Audit()
    files = _files(out)
    features = set(schema.SPEECH_SCHEMA.names)
    _file_checks(a, files, features)
    for f in files[:1]:
        meta = json.loads(pq.ParquetFile(f).schema_arrow.metadata[b"huggingface"])["info"][
            "features"
        ]
        a.check(
            meta["audio"] == {"sampling_rate": 16000, "_type": "Audio"},
            "audio declared as Audio, 16 kHz",
        )
    donors = _donors()
    print("rows (every clip decoded)")
    meta_rows, durations_off, undecodable, wrong_format = [], [], [], []
    random.seed(0)
    compared = mismatched = 0
    for f in files:
        pf = pq.ParquetFile(f)
        for i in range(pf.metadata.num_row_groups):
            for r in pf.read_row_group(i).to_pylist():
                b = r["audio"]["bytes"]
                try:
                    info = sf.info(io.BytesIO(b))
                    if info.samplerate != 16000 or info.channels != 1 or info.format != "FLAC":
                        wrong_format.append(r["id"])
                    if abs(info.frames / 16000 - r["duration_s"]) > 0.011:
                        durations_off.append(r["id"])
                except Exception:  # noqa: BLE001
                    undecodable.append(r["id"])
                if random.random() < sample / max(1, pf.metadata.num_rows * len(files)):
                    audio, _ = sf.read(io.BytesIO(b), dtype="int16")
                    src = clips / f"{r['rendition_id']}.wav"
                    if src.is_file():
                        compared += 1
                        mismatched += not (audio == sf.read(src, dtype="int16")[0]).all()
                r["_audio_path"] = r.pop("audio")["path"]
                r["_split"] = Path(f).name.split("-")[0]
                meta_rows.append(r)
    a.check(not undecodable, "every clip decodes", f"{len(undecodable)}")
    a.check(not wrong_format, "every clip is FLAC, 16 kHz, mono", f"{len(wrong_format)}")
    a.check(
        not durations_off,
        "duration_s matches the audio",
        f"{len(durations_off)} e.g. {durations_off[:2]}",
    )
    a.check(
        compared > 0 and mismatched == 0,
        "sampled clips identical to their source WAV",
        f"{compared} compared, {mismatched} differ",
    )
    rows = meta_rows
    _no_empty(
        a,
        rows,
        [
            "id",
            "rendition_id",
            "language",
            "transcript",
            "final_text",
            "category",
            "domain",
            "tts_engine",
            "tts_voice",
            "collection",
            "source",
            "source_url",
            "license",
        ],
        "all",
    )
    a.check(
        all(r["duration_s"] and r["duration_s"] > 0.5 for r in rows),
        "durations positive and above 0.5 s",
        f"min {min(r['duration_s'] for r in rows):.2f} s, max {max(r['duration_s'] for r in rows):.1f} s",
    )
    a.check(len({r["id"] for r in rows}) == len(rows), "ids unique", f"{len(rows)} rows")
    a.check(
        all(r["id"] == f"{r['collection']}/{r['rendition_id']}" for r in rows),
        "id = collection/rendition_id",
    )
    a.check({r["language"] for r in rows} == {"en"}, "language en")
    a.check({r["category"] for r in rows} <= set(schema.CATEGORIES), "categories known")
    a.check({r["license"] for r in rows} <= schema.PUBLISHED_LICENSES, "licenses known")
    a.check({r["source"] for r in rows} <= schema.SOURCES, "sources known")
    a.check(all(r["source_url"].startswith("https://") for r in rows), "source URLs are https")
    a.check(
        all(re.fullmatch(r"[\w:]+=[^=]+", a) for r in rows for a in r["annotations"]),
        "annotations are key=value",
    )
    a.check(
        all(
            r["tts_settings"]
            and all(
                re.fullmatch(r"(speed|exaggeration)=[\d.]+|audio=regenerated", s)
                for s in r["tts_settings"]
            )
            for r in rows
        ),
        "tts_settings well formed",
    )
    a.check(
        all(
            ("speed=" in r["tts_settings"][0]) == r["tts_engine"].startswith("kokoro") for r in rows
        ),
        "speed for Kokoro, exaggeration for Chatterbox",
    )
    a.check(
        all(not r["tts_voice"].startswith("voice-donations") for r in rows),
        "no donated voice file names",
    )
    # The fields Utter's tools wrote (not the passages' own text, which may mention paths or names).
    ours = json.dumps(
        [
            [
                r["_audio_path"],
                r["tts_voice"],
                r["tts_settings"],
                r["annotations"],
                r["collection"],
                r["source_url"],
            ]
            for r in rows
        ],
        ensure_ascii=False,
    )
    # Donor names: in the fields about the speech (a source URL is the passage's own page, and a
    # Wikipedia title can share a first name).
    speech = json.dumps(
        [
            [r["_audio_path"], r["tts_voice"], r["tts_settings"], r["annotations"], r["collection"]]
            for r in rows
        ]
    )
    leaked = sorted(
        d for d in donors if re.search(r"\b" + re.escape(d) + r"\b", speech, re.IGNORECASE)
    )
    a.check(
        not leaked,
        "no donor name in the generated fields",
        f"{len(donors)} names checked; found {leaked}",
    )
    a.check(
        not re.search(r"[A-Za-z]:\\\\|/home/|/workspace/|Users\\\\|tdn47", ours),
        "no machine paths in the generated fields",
    )
    a.check(
        all(re.fullmatch(r"[0-9a-f]{12}-[a-z]+\.flac", r["_audio_path"]) for r in rows),
        "audio paths are bare <rendition>.flac",
    )
    # Across every shard from here on: the Hub's and the new ones.
    remote = hub.speech_columns(
        config.SPEECH_REPO, rev, ["id", "rendition_id", "collection", "final_text"]
    )
    everything = [{**r, "_split": r.pop("split")} for r in remote] + rows
    a.check(
        len({r["id"] for r in everything}) == len(everything),
        "ids unique across the dataset",
        f"{len(everything)} rows",
    )
    splits = collections.defaultdict(set)
    passages = collections.defaultdict(set)
    for r in everything:
        split = r["_split"]
        splits[split].add(r["rendition_id"])
        passages[split].add(schema.passage_id(r["rendition_id"]))
    a.check(
        not (splits["train"] & splits["test"]),
        "no rendition in both splits",
        f"{len(splits['train'] & splits['test'])}",
    )
    a.check(
        not (passages["train"] & passages["test"]),
        "no passage in both splits",
        f"{len(passages['train'] & passages['test'])}",
    )
    bad = [r["id"] for r in rows if schema.fix_final(r["final_text"]) != r["final_text"]]
    a.check(not bad, "no fixable punctuation slip left in final_text", f"{len(bad)}")
    print(
        f"  [info] regenerated clips: {sum('audio=regenerated' in r['tts_settings'] for r in rows)}"
    )
    print("against dictation-to-text")
    trows = {t["id"]: t for t in text}
    corpus = [r for r in everything if r["collection"].startswith(schema.CORPUS_PREFIX)]
    unpaired = [r["id"] for r in corpus if r["rendition_id"] not in trows]
    a.check(not unpaired, "every corpus clip has its text pair", f"{len(unpaired)}")
    per_rendition = collections.Counter(r["rendition_id"] for r in corpus)
    missing = [i for i in trows if per_rendition[i] != 1]
    a.check(
        not missing,
        "every text pair has exactly one corpus clip",
        f"{len(missing)} e.g. {missing[:2]}",
    )
    joined = [r for r in corpus if r["rendition_id"] in trows]
    other = [r["id"] for r in joined if trows[r["rendition_id"]]["_split"] != r["_split"]]
    a.check(not other, "each clip in its text pair's split", f"{len(other)}")
    listed = [r for r in joined if trows[r["rendition_id"]]["lists"]]
    diff = [
        r["rendition_id"]
        for r in listed
        if trows[r["rendition_id"]]["final_text"] != r["final_text"]
    ]
    a.check(
        not diff,
        "final_text identical in both datasets (lists on)",
        f"{len(diff)} of {len(listed)} e.g. {diff[:2]}",
    )
    print(f"  [info] lists-off pairs, whose clip keeps the list form: {len(joined) - len(listed)}")
    print()
    # The card covers every shard: the new ones and the Hub's.
    expected = collections.Counter(r["_split"] for r in rows)
    for sh in json.loads((out / "hub-shards.json").read_text(encoding="utf-8")).values():
        expected[sh["split"]] += sh["rows"]
    _card_checks(a, out, dict(expected))
    return a.failures
