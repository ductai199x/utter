"""A batch: one round of new data, from passages to publishable rows, in `<data dir>/data/<name>/`.

  batch.json        its name, the commit that made it, its seed, its collection, whether test-only
  passages.jsonl    id, domain, final_text, source, source_url, license
  renditions.jsonl  one per spoken version: a speech row without audio (id = the rendition id)
  text.jsonl        text rows, once the clips are transcribed

Its clips are a speech collection under the eval dir (`<eval dir>/<collection>/`), the layout
`utter-cli bench` reads. Every file holds the datasets' own fields, so publishing merges them as
they are. A passage already published is never added again.
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path

from utter_tools import common, schema


@dataclass
class Meta:
    name: str
    commit: str
    seed: int
    collection: str
    # An eval set: speech only, every clip in test (as the published eval sets are).
    test_only: bool


class Batch:
    def __init__(self, name: str) -> None:
        self.root = common.data_dir() / "data" / name
        meta = self.root / "batch.json"
        if not meta.is_file():
            raise FileNotFoundError(f"no batch {name}: utter-data new {name}")
        self.meta = Meta(**json.loads(meta.read_text()))
        self.passages = self.root / "passages.jsonl"
        self.renditions = self.root / "renditions.jsonl"
        self.text = self.root / "text.jsonl"
        self.clips = common.eval_dir() / self.meta.collection

    @staticmethod
    def create(name: str, seed: int, collection: str, test_only: bool) -> Batch:
        root = common.data_dir() / "data" / name
        if root.exists():
            raise FileExistsError(f"batch {name} exists")
        head = ["git", "-C", str(common.REPO), "rev-parse", "HEAD"]
        commit = subprocess.run(head, capture_output=True, text=True, check=True).stdout.strip()
        root.mkdir(parents=True)
        meta = Meta(name, commit, seed, collection, test_only)
        (root / "batch.json").write_text(json.dumps(asdict(meta), indent=2))
        return Batch(name)

    def code_url(self, module: str) -> str:
        """Where a synthetic passage came from: the module that wrote it, at the batch's commit."""
        repo = f"https://github.com/ductai199x/utter/blob/{self.meta.commit}"
        return f"{repo}/tools/utter_tools/data/{module}.py"

    def add_passages(self, rows: list[dict]) -> int:
        """Adds the passages that are new to the batch and to both published datasets."""
        have = published_passages()
        for p in common.read_jsonl(self.passages):
            have |= {p["id"], normalized(p["final_text"])}
        new = []
        for p in rows:
            key = normalized(p["final_text"])
            if p["id"] not in have and key not in have:
                have |= {p["id"], key}
                new.append(p)
        common.write_jsonl(self.passages, new, append=True)
        return len(new)


def normalized(text: str) -> str:
    """Text compared for overlap: case and spacing do not make a passage new."""
    return " ".join(text.lower().split())


def published_passages() -> set[str]:
    """Passage ids and normalized final texts in both datasets (the eval sets are speech only)."""
    out: set[str] = set()
    text_rev = common.revision(common.TEXT_REPO)
    for split in ("train", "test"):
        for r in common.text_rows(text_rev, split, ["id", "final_text"]):
            out |= {schema.passage_id(r["id"]), normalized(r["final_text"])}
    speech_rev = common.revision(common.SPEECH_REPO)
    for r in common.speech_columns(speech_rev, ["rendition_id", "final_text"]):
        out |= {schema.passage_id(r["rendition_id"]), normalized(r["final_text"])}
    return out


def passage(domain: str, text: str, source: str, source_url: str, license_raw: str) -> dict | None:
    """A passage row, or None when its license is not one the datasets publish or it has no URL
    to credit."""
    license_ = schema.license_id(license_raw)
    if license_ is None or not source_url:
        return None
    return {
        "id": schema.new_passage_id(text),
        "domain": domain,
        "final_text": schema.fix_final(text),
        "source": source,
        "source_url": source_url,
        "license": license_,
    }


def ids(path: Path) -> set[str]:
    return {r["id"] for r in common.read_jsonl(path)}
