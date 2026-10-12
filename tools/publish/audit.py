"""Audits a built dataset folder before upload: every row and every file, against the rules the
builders promise. Prints each check and exits non-zero on any failure.

Run (from tools): uv run python -m publish.audit text OUT_DIR
                  uv run python -m publish.audit voice OUT_DIR [--wav-root EVAL_DIR] [--text TEXT_DIR]
"""

from __future__ import annotations

import collections
import glob
import io
import json
import random
import re
import sys
from pathlib import Path

import pyarrow.parquet as pq
import soundfile as sf
import typer

from publish.common import fix_final, jsonl

CATEGORIES = {
    "natural",
    "disfluent",
    "correction",
    "command",
    "explicit",
    "amend",
    "list",
    "grammar",
    "restate",
}
# Only what the datasets may contain (publish/common.py EXCLUDED_SOURCES): share-alike, CC0, public domain.
LICENSES = {"cc-by-sa-4.0", "cc-by-sa-3.0", "cc-by-sa-2.5", "cc0-1.0", "public-domain"}
SOURCES = {
    "common-pile/stackexchange",
    "common-pile/arxiv_abstracts",
    "common-pile/ubuntu_irc",
    "wikimedia/wikipedia",
    "synthetic",
}
MAX_ROW_GROUP = 300e6

failures: list[str] = []


def check(ok: bool, what: str, detail: str = "") -> None:
    print(f"  [{'ok' if ok else 'FAIL'}] {what}{(': ' + detail) if detail else ''}")
    if not ok:
        failures.append(what)


def _files(out: Path) -> list[str]:
    return sorted(glob.glob(str(out / "data" / "**" / "*.parquet"), recursive=True))


def _file_checks(files: list[str], expect_features: set[str]) -> None:
    print("files")
    for f in files:
        pf = pq.ParquetFile(f)
        meta = json.loads(pf.schema_arrow.metadata[b"huggingface"])["info"]["features"]
        md = pf.metadata
        rg = max(md.row_group(i).total_byte_size for i in range(md.num_row_groups))
        name = Path(f).name
        check(
            set(meta) == expect_features == set(pf.schema_arrow.names),
            f"{name}: features declared",
            f"{len(meta)} columns",
        )
        check(
            rg <= MAX_ROW_GROUP, f"{name}: row groups at most 300 MB", f"largest {rg / 1e6:.0f} MB"
        )
        check(
            re.fullmatch(r"(train|test)-\d{5}-of-\d{5}\.parquet", name) is not None,
            f"{name}: Hub file name",
        )
    print()


def _card_checks(out: Path, expected: dict[str, int]) -> None:
    from huggingface_hub import DatasetCard

    print("card")
    card = out / "README.md"
    if not card.is_file():
        print("  [info] no README.md yet: the card is checked once it is written")
        return
    c = DatasetCard.load(card)
    try:
        c.validate(repo_type="dataset")
        check(True, "YAML validates against the Hub's schema")
    except Exception as e:  # noqa: BLE001
        check(False, "YAML validates against the Hub's schema", str(e)[:200])
    splits = {s["name"]: s["num_examples"] for s in c.data.to_dict()["dataset_info"]["splits"]}
    check(splits == expected, "card split counts match the files", f"{splits}")
    print()


def _no_empty(rows: list[dict], columns: list[str], where: str) -> None:
    for col in columns:
        bad = [r["id"] for r in rows if r[col] is None or r[col] == ""]
        check(not bad, f"{where}: no empty {col}", f"{len(bad)} e.g. {bad[:2]}" if bad else "")


def text(out: Path) -> None:
    files = _files(out)
    _file_checks(
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
        allrows,
        ["id", "category", "domain", "dictated", "final_text", "source", "source_url", "license"],
        "all",
    )
    check(
        all(
            r["dictionary"] is not None and r["lists"] is not None and r["messages"]
            for r in allrows
        ),
        "list and bool columns never null",
    )
    check(
        len({r["id"] for r in allrows}) == len(allrows),
        "ids unique across splits",
        f"{len(allrows)} rows",
    )
    check(
        {r["category"] for r in allrows} <= CATEGORIES,
        "categories known",
        str(sorted({r["category"] for r in allrows})),
    )
    check(
        {r["license"] for r in allrows} <= LICENSES,
        "licenses known",
        f"{len({r['license'] for r in allrows})} ids",
    )
    check({r["source"] for r in allrows} <= SOURCES, "sources known")
    check(all(r["source_url"].startswith("https://") for r in allrows), "source URLs are https")
    check(all(r["id"].endswith("-" + r["category"]) for r in allrows), "id = passage id + category")
    bad = [
        r["id"]
        for r in allrows
        if [m["role"] for m in r["messages"]] != ["system", "user", "assistant"]
    ]
    check(not bad, "messages are system, user, assistant", f"{bad[:2]}")
    bad = [r["id"] for r in allrows if r["messages"][2]["content"] != r["final_text"]]
    check(not bad, "the assistant message is final_text", f"{len(bad)}")
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
    check(
        not bad,
        "dictated, dictionary and lists agree with the user message",
        f"{len(bad)} e.g. {bad[:2]}",
    )
    check(len({r["messages"][0]["content"] for r in allrows}) == 1, "one system prompt")
    bad = [r["id"] for r in allrows if fix_final(r["final_text"]) != r["final_text"]]
    check(not bad, "no fixable punctuation slip left in final_text", f"{len(bad)}")
    trp = {r["id"].split("-")[0] for r in split_rows["train"]}
    leak = [r["id"] for r in split_rows["test"] if r["id"].split("-")[0] in trp]
    check(not leak, "no test passage in train", f"{len(leak)}")
    dup = collections.Counter((r["dictated"], r["final_text"]) for r in allrows)
    n = sum(c - 1 for c in dup.values() if c > 1)
    print(f"  [info] exact duplicate (dictated, final_text) pairs: {n}")
    print()
    _card_checks(out, {s: len(v) for s, v in split_rows.items()})


def voice(
    out: Path,
    wav_root: Path = typer.Option(None),
    text_dir: Path = typer.Option(None, "--text"),
    sample: int = 300,
) -> None:
    files = _files(out)
    features = {
        "id",
        "rendition_id",
        "audio",
        "duration_s",
        "language",
        "transcript",
        "final_text",
        "category",
        "domain",
        "annotations",
        "tts_engine",
        "tts_voice",
        "tts_settings",
        "collection",
        "source",
        "source_url",
        "license",
    }
    _file_checks(files, features)
    for f in files[:1]:
        meta = json.loads(pq.ParquetFile(f).schema_arrow.metadata[b"huggingface"])["info"][
            "features"
        ]
        check(
            meta["audio"] == {"sampling_rate": 16000, "_type": "Audio"},
            "audio declared as Audio, 16 kHz",
        )
    donors = set()
    if wav_root:
        for m in glob.glob(str(wav_root / "en-finaltext-chatterbox*" / "manifest.jsonl")):
            for row in jsonl(m):
                donors.add(Path(row["voice"]).stem.replace("_enhanced", ""))
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
                if wav_root and random.random() < sample / 120_000:
                    audio, _ = sf.read(io.BytesIO(b), dtype="int16")
                    name = r["collection"].replace("corpus", "train-kokoro", 1)
                    folder = "en-finaltext-" + name
                    cand = [
                        wav_root / d / (r["rendition_id"] + ".wav")
                        for d in (
                            folder,
                            folder.replace("train-kokoro", "train-kokoro2"),
                            "en-finaltext-train-kokoro-respoken",
                        )
                    ]
                    src = next((c for c in cand if c.is_file()), None)
                    if src is not None:
                        compared += 1
                        mismatched += not (audio == sf.read(src, dtype="int16")[0]).all()
                r["_audio_path"] = r.pop("audio")["path"]
                r["_split"] = Path(f).name.split("-")[0]
                meta_rows.append(r)
    check(not undecodable, "every clip decodes", f"{len(undecodable)}")
    check(not wrong_format, "every clip is FLAC, 16 kHz, mono", f"{len(wrong_format)}")
    check(
        not durations_off,
        "duration_s matches the audio",
        f"{len(durations_off)} e.g. {durations_off[:2]}",
    )
    if wav_root:
        check(
            compared > 0 and mismatched == 0,
            "sampled clips identical to their source WAV",
            f"{compared} compared, {mismatched} differ",
        )
    rows = meta_rows
    _no_empty(
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
    check(
        all(r["duration_s"] and r["duration_s"] > 0.5 for r in rows),
        "durations positive and above 0.5 s",
        f"min {min(r['duration_s'] for r in rows):.2f} s, max {max(r['duration_s'] for r in rows):.1f} s",
    )
    check(len({r["id"] for r in rows}) == len(rows), "ids unique", f"{len(rows)} rows")
    check(
        all(r["id"] == f"{r['collection']}/{r['rendition_id']}" for r in rows),
        "id = collection/rendition_id",
    )
    check({r["language"] for r in rows} == {"en"}, "language en")
    check({r["category"] for r in rows} <= CATEGORIES, "categories known")
    check({r["license"] for r in rows} <= LICENSES, "licenses known")
    check({r["source"] for r in rows} <= SOURCES, "sources known")
    check(all(r["source_url"].startswith("https://") for r in rows), "source URLs are https")
    check(
        all(re.fullmatch(r"[\w:]+=[^=]+", a) for r in rows for a in r["annotations"]),
        "annotations are key=value",
    )
    check(
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
    check(
        all(
            ("speed=" in r["tts_settings"][0]) == r["tts_engine"].startswith("kokoro") for r in rows
        ),
        "speed for Kokoro, exaggeration for Chatterbox",
    )
    check(
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
    check(
        not leaked,
        "no donor name in the generated fields",
        f"{len(donors)} names checked; found {leaked}",
    )
    check(
        not re.search(r"[A-Za-z]:\\\\|/home/|/workspace/|Users\\\\|tdn47", ours),
        "no machine paths in the generated fields",
    )
    check(
        all(re.fullmatch(r"[0-9a-f]{12}-[a-z]+\.flac", r["_audio_path"]) for r in rows),
        "audio paths are bare <rendition>.flac",
    )
    splits = collections.defaultdict(set)
    passages = collections.defaultdict(set)
    for r in rows:
        split = r["_split"]
        splits[split].add(r["rendition_id"])
        passages[split].add(r["rendition_id"].split("-")[0])
    check(
        not (splits["train"] & splits["test"]),
        "no rendition in both splits",
        f"{len(splits['train'] & splits['test'])}",
    )
    check(
        not (passages["train"] & passages["test"]),
        "no passage in both splits",
        f"{len(passages['train'] & passages['test'])}",
    )
    bad = [r["id"] for r in rows if fix_final(r["final_text"]) != r["final_text"]]
    check(not bad, "no fixable punctuation slip left in final_text", f"{len(bad)}")
    print(
        f"  [info] regenerated clips: {sum('audio=regenerated' in r['tts_settings'] for r in rows)}"
    )
    if text_dir:
        print("against dictation-to-text")
        trows = {}
        for f in _files(text_dir):
            for t in pq.read_table(f, columns=["id", "final_text", "lists"]).to_pylist():
                t["_split"] = Path(f).name.split("-")[0]
                trows[t["id"]] = t
        corpus = [r for r in rows if r["collection"].startswith("corpus")]
        unpaired = [r["id"] for r in corpus if r["rendition_id"] not in trows]
        check(not unpaired, "every corpus clip has its text pair", f"{len(unpaired)}")
        clips = collections.Counter(r["rendition_id"] for r in corpus)
        missing = [i for i in trows if clips[i] != 1]
        check(
            not missing,
            "every text pair has exactly one corpus clip",
            f"{len(missing)} e.g. {missing[:2]}",
        )
        joined = [r for r in corpus if r["rendition_id"] in trows]
        other = [r["id"] for r in joined if trows[r["rendition_id"]]["_split"] != r["_split"]]
        check(not other, "each clip in its text pair's split", f"{len(other)}")
        listed = [r for r in joined if trows[r["rendition_id"]]["lists"]]
        diff = [
            r["rendition_id"]
            for r in listed
            if trows[r["rendition_id"]]["final_text"] != r["final_text"]
        ]
        check(
            not diff,
            "final_text identical in both datasets (lists on)",
            f"{len(diff)} of {len(listed)} e.g. {diff[:2]}",
        )
        print(
            f"  [info] lists-off pairs, whose clip keeps the list form: {len(joined) - len(listed)}"
        )
    print()
    # The card covers every shard: these here, and those summarized where they live (card summary).
    expected = collections.Counter(r["_split"] for r in rows)
    for f in sorted(out.glob("summary-*.json")):
        for key, sh in json.loads(f.read_text(encoding="utf-8")).items():
            if not (out / "data" / key).is_file():
                expected[sh["split"]] += sh["rows"]
    _card_checks(out, dict(expected))


app = typer.Typer()
app.command("text")(text)
app.command("voice")(voice)

if __name__ == "__main__":
    app()
    if failures:
        print(f"{len(failures)} FAILED")
        sys.exit(1)
