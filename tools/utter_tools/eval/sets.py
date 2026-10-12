"""The cleanup eval's sets, made from the published datasets' test splits at one revision:

- every speech collection but the corpus (the eval sets: realistic dictation, directed text,
  restate, grammar, lists, far-back edits, with Kokoro voices and with Chatterbox voices no
  training clip uses), as `<eval dir>/<collection>/`: a manifest and 16 kHz WAVs, the layout
  `utter-cli bench` reads;
- the text dataset's test split as `<eval dir>/text-test/`, with what the recognizer heard (its
  `dictated`) as the recognizer run `dictated`.

Each manifest row is the dataset's own row, with the schema's field names (schema.py): a speech
row with its audio as a WAV beside it (`utter-cli bench` reads `id`, `audio`, `transcript`,
`language`, `duration_s`), a text row without its training chat. A set already made from the same
revision is kept.
"""

from __future__ import annotations

import io
import json
from pathlib import Path

import soundfile as sf

from utter_tools import config, hub, schema

TEXT_SET = "text-test"
# The recognizer "run" of the text test split: the transcript the dataset records.
DICTATED = "dictated"


def _write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".part")
    with tmp.open("w", encoding="utf-8", newline="\n") as fh:
        for r in rows:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
    tmp.replace(path)


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def asr_path(run: str, eval_set: str) -> Path:
    """A recognizer run's transcripts of a set."""
    return config.results_dir() / "asr" / run / f"{eval_set}.jsonl"


def llm_path(run: str, eval_set: str) -> Path:
    """A cleanup run's outputs on a set (utter-cli llm-bench writes them over its cases)."""
    return config.results_dir() / "llm" / run / f"{eval_set}-input.jsonl"


def manifest(eval_set: str) -> list[dict]:
    return read_jsonl(config.eval_dir() / eval_set / "manifest.jsonl")


def _current(folder: Path, repo: str, rev: str) -> bool:
    source = folder / "source.json"
    return source.is_file() and json.loads(source.read_text()) == {"repo": repo, "revision": rev}


def _mark(folder: Path, repo: str, rev: str) -> None:
    (folder / "source.json").write_text(json.dumps({"repo": repo, "revision": rev}))


def make_text_set(rev: str) -> None:
    folder = config.eval_dir() / TEXT_SET
    if _current(folder, config.TEXT_REPO, rev):
        return
    columns = [f for f in schema.TEXT_SCHEMA.names if f != "messages"]
    rows = hub.text_rows(config.TEXT_REPO, rev, "test", columns)
    _write_jsonl(folder / "manifest.jsonl", rows)
    _write_jsonl(
        asr_path(DICTATED, TEXT_SET), [{"id": r["id"], "hyp": r["dictated"]} for r in rows]
    )
    _mark(folder, config.TEXT_REPO, rev)
    print(f"{TEXT_SET}: {len(rows)} rows")


def make_speech_set(collection: str, rev: str) -> None:
    folder = config.eval_dir() / collection
    if _current(folder, config.SPEECH_REPO, rev):
        return
    folder.mkdir(parents=True, exist_ok=True)
    rows = []
    for r in hub.speech_rows(config.SPEECH_REPO, rev, collection, "test"):
        audio, rate = sf.read(io.BytesIO(r["audio"]["bytes"]), dtype="int16")
        if rate != schema.RATE:
            raise ValueError(f"{r['id']}: {rate} Hz")
        wav = f"{r['rendition_id']}.wav"
        sf.write(folder / wav, audio, rate, subtype="PCM_16")
        rows.append({**r, "audio": wav})
    _write_jsonl(folder / "manifest.jsonl", rows)
    _mark(folder, config.SPEECH_REPO, rev)
    print(f"{collection}: {len(rows)} clips")


def speech_sets(rev: str) -> list[str]:
    return [
        c
        for c in hub.speech_collections(config.SPEECH_REPO, rev)
        # The corpus's test clips are the text test split's rows, which text-test covers.
        if not c.startswith(schema.CORPUS_PREFIX)
    ]


def make(text_ref: str | None = None, speech_ref: str | None = None) -> dict[str, str]:
    """Makes every set from the datasets at `text_ref` and `speech_ref` (default: main); returns
    the revisions used."""
    text_rev = hub.revision(config.TEXT_REPO, text_ref)
    speech_rev = hub.revision(config.SPEECH_REPO, speech_ref)
    make_text_set(text_rev)
    for c in speech_sets(speech_rev):
        make_speech_set(c, speech_rev)
    return {config.TEXT_REPO: text_rev, config.SPEECH_REPO: speech_rev}
