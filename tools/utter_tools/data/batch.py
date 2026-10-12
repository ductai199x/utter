"""A batch: one round of new data, from passages to publishable rows, in `<data dir>/data/<name>/`.

  batch.json          name, the commit that made it, its seed, whether it is test-only
  passages.jsonl      passages: id, domain, final_text, source, source_url, license (schema.py)
  renditions.jsonl    one row per spoken version: a speech row without audio (id is the
                      rendition id; transcript is what is said)
  text.jsonl          text rows (dictation-to-text's schema), once the clips are transcribed

Its clips are a speech collection under the eval dir (`<eval dir>/<collection>/`, the layout
`utter-cli bench` reads), so the recognizer runs on them as on any eval set.

Every file holds the datasets' own fields: publishing (data.publish) merges them into the Hub
datasets as they are. Passages already published are never added again.
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path

from utter_tools import config, hub, schema


def read_jsonl(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    with path.open(encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".part")
    with tmp.open("w", encoding="utf-8", newline="\n") as fh:
        for r in rows:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
    tmp.replace(path)


def append_jsonl(path: Path, row: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8", newline="\n") as fh:
        fh.write(json.dumps(row, ensure_ascii=False) + "\n")


def commit() -> str:
    """The repository's commit, so a synthetic passage's source_url points at the code that wrote it."""
    out = subprocess.run(
        ["git", "-C", str(config.REPO), "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
    return out.stdout.strip()


def code_url(module: str, rev: str) -> str:
    """The URL of a tools module at a commit (a synthetic passage's source_url)."""
    return f"https://github.com/ductai199x/utter/blob/{rev}/tools/utter_tools/data/{module}.py"


@dataclass
class Meta:
    name: str
    commit: str
    seed: int
    # Every passage goes to test (an eval set); else publishing splits them as it splits all.
    test_only: bool
    # The speech collection its clips are published as.
    collection: str


class Batch:
    def __init__(self, name: str) -> None:
        self.root = config.work_dir() / name
        if not (self.root / "batch.json").is_file():
            raise FileNotFoundError(f"no batch {name}: utter-data new {name}")
        self.meta = Meta(**json.loads((self.root / "batch.json").read_text()))

    @staticmethod
    def create(name: str, seed: int, test_only: bool, collection: str) -> Batch:
        root = config.work_dir() / name
        if (root / "batch.json").exists():
            raise FileExistsError(f"batch {name} exists")
        root.mkdir(parents=True)
        meta = Meta(name, commit(), seed, test_only, collection)
        (root / "batch.json").write_text(json.dumps(asdict(meta), indent=2))
        return Batch(name)

    @property
    def passages_path(self) -> Path:
        return self.root / "passages.jsonl"

    @property
    def renditions_path(self) -> Path:
        return self.root / "renditions.jsonl"

    @property
    def text_path(self) -> Path:
        return self.root / "text.jsonl"

    @property
    def clips(self) -> Path:
        """The batch's speech collection, where utter-cli bench reads it."""
        return config.eval_dir() / self.meta.collection

    def passages(self) -> list[dict]:
        return read_jsonl(self.passages_path)

    def add_passages(self, rows: list[dict], published: set[str]) -> int:
        """Adds passages not in the batch or the published datasets (`published`: ids and
        normalized texts); returns how many were new."""
        have = {p["id"] for p in self.passages()} | published
        added = 0
        for p in rows:
            key = normalized(p["final_text"])
            if p["id"] in have or key in have:
                continue
            have |= {p["id"], key}
            append_jsonl(self.passages_path, p)
            added += 1
        return added


def normalized(text: str) -> str:
    """Text compared for overlap: case and spacing do not make a passage new."""
    return " ".join(text.lower().split())


def published_passages() -> set[str]:
    """Passage ids and normalized final texts in both datasets: the text dataset's rows, and the
    speech dataset's (its eval sets are speech only)."""
    out: set[str] = set()
    text_rev = hub.revision(config.TEXT_REPO)
    for split in ("train", "test"):
        for r in hub.text_rows(config.TEXT_REPO, text_rev, split, ["id", "final_text"]):
            out |= {schema.passage_id(r["id"]), normalized(r["final_text"])}
    for r in hub.speech_columns(
        config.SPEECH_REPO, hub.revision(config.SPEECH_REPO), ["rendition_id", "final_text"]
    ):
        out |= {schema.passage_id(r["rendition_id"]), normalized(r["final_text"])}
    return out


def passage(
    pid: str, domain: str, text: str, source: str, source_url: str, license_raw: str
) -> dict:
    """A passage row, with its license as the datasets' id."""
    return {
        "id": pid,
        "domain": domain,
        "final_text": schema.fix_final(text),
        "source": source,
        "source_url": source_url,
        "license": schema.license_id(license_raw),
    }
