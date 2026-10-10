"""Lists for the "list" rendition kind (docs/cleanup-spec.md, rule 4; docs/plan-use-cases.md,
recipe R3): spoken enumerations become bulleted lists, or numbered ones for steps, unless the
user turns lists off.

Golds are real lists people wrote (Stack Exchange and GitHub, split by document as the passages
are), extracted with the line that introduces them. The spoken script is built by code from the
list, in the ways people enumerate aloud: plainly ("eggs, milk, bread and coffee"), with cues
("first... second...", "bullet point..."), or with "then... finally..." for steps. With lists
off, the gold is the same content as prose, made from the same script.

Run: uv run python -m eval.finaltext.lists extract --split train --per-shard 400
"""

from __future__ import annotations

import hashlib
import itertools
import json
import random
import re
from dataclasses import dataclass, field
from pathlib import Path

import typer

from eval.finaltext.sources import _CODE, _LATEX, _URL, SHARDS, documents, english, split_of
from eval.paths import eval_dir

app = typer.Typer(add_completion=False)


@app.callback()
def _commands() -> None:
    """Lists for the final-text eval and training data."""


_ITEM = re.compile(r"^[ \t]*(?:([-*•])|(\d+)[.)])[ \t]+(\S.*)$")
_ORDINALS = [
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
]
_NUMBERS = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"]


@dataclass
class ListPassage:
    intro: str
    items: list[str]
    ordered: bool
    outro: str = ""
    tags: dict[str, str] = field(default_factory=dict)

    def markdown(self) -> str:
        marks = [f"{i + 1}. " if self.ordered else "- " for i in range(len(self.items))]
        lines = [m + it for m, it in zip(marks, self.items, strict=True)]
        out = (self.intro + "\n" if self.intro else "") + "\n".join(lines)
        return out + ("\n\n" + self.outro if self.outro else "")


def parse(text: str) -> ListPassage | None:
    """A gold back into its parts: intro line, items, numbered or not, closing sentence."""
    lines = text.split("\n")
    idx = [i for i, ln in enumerate(lines) if _ITEM.match(ln)]
    if len(idx) < 2 or idx != list(range(idx[0], idx[-1] + 1)):
        return None
    ordered = bool(_ITEM.match(lines[idx[0]]).group(2))  # type: ignore[union-attr]
    items = [_ITEM.match(lines[i]).group(3).strip() for i in idx]  # type: ignore[union-attr]
    intro = " ".join(ln.strip() for ln in lines[: idx[0]] if ln.strip())
    outro = " ".join(ln.strip() for ln in lines[idx[-1] + 1 :] if ln.strip())
    return ListPassage(intro, items, ordered, outro)


# ---------------------------------------------------------------------------------------------
# Extraction from the sources.


# Shell and code fragments the code filter misses: commands, flags, operators.
_SHELL = re.compile(
    r"(?:^|\s)(?:sudo|apt(?:-get)?|dpkg|chmod|chown|chgrp|grep|cd|ls|rm|mv|cp|fi|done|esac)\b"
    r"|\s--?[a-z]|[+=_/\\|<>{}$]"
)


def _dictatable_item(item: str) -> bool:
    words = item.split()
    if not 1 <= len(words) <= 20:
        return False
    if _URL.search(item) or _CODE.search(item) or _LATEX.search(item) or _SHELL.search(item):
        return False
    letters = sum(c.isalpha() for c in item)
    return letters / max(1, sum(not c.isspace() for c in item)) >= 0.75 and item.isascii()


def consistent(items: list[str]) -> list[str]:
    """Items written one way: a final period on all or none, and a capital first letter on all
    or none, as most of them have it. A careful writer is consistent, and the gold is what a
    careful writer types."""
    stripped = [it.rstrip(".;,") for it in items]
    period = sum(it.endswith(".") for it in items) * 2 > len(items)
    capital = sum(it[:1].isupper() for it in stripped) * 2 > len(items)

    def first(it: str) -> str:
        head = it.split(" ", 1)[0]
        # Acronyms and names keep their case ("GPU", "iPhone").
        if head[1:] != head[1:].lower() or (head.isupper() and len(head) > 1):
            return it
        return (it[0].upper() if capital else it[0].lower()) + it[1:]

    return [first(it) + ("." if period else "") for it in stripped]


def lists_in(text: str) -> list[ListPassage]:
    """List blocks of 3-10 dictatable items in a document, each with the sentence before it
    when that sentence introduces it (ends with a colon)."""
    out = []
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        if not _ITEM.match(lines[i]):
            i += 1
            continue
        j = i
        while j < len(lines) and _ITEM.match(lines[j]):
            j += 1
        block = [_ITEM.match(ln) for ln in lines[i:j]]
        items = [m.group(3).strip() for m in block if m]
        ordered = all(m.group(2) for m in block if m)
        unordered = all(m.group(1) for m in block if m)
        prev = lines[i - 1].strip() if i > 0 else ""
        intro = (
            prev
            if prev.endswith(":") and len(prev.split()) <= 25 and _dictatable_item(prev[:-1])
            else ""
        )
        if 3 <= len(items) <= 10 and (ordered or unordered) and all(map(_dictatable_item, items)):
            lp = ListPassage(intro, consistent(items), ordered)
            if english(lp.markdown()):
                out.append(lp)
        i = j
    return out


@app.command()
def extract(
    split: str = typer.Option("train", "--split"),
    per_shard: int = 400,
    seed: int = 0,
) -> None:
    """Writes passages-lists-<split>.jsonl from the Stack Exchange and GitHub shards."""
    from huggingface_hub import hf_hub_download

    rng = random.Random(seed)
    rows, seen = [], set()
    for domain, repo, file, kind in SHARDS:
        if kind not in ("se", "gh"):
            continue
        path = Path(hf_hub_download(repo, file, repo_type="dataset"))
        found = []
        for doc_id, text, url, lic in itertools.islice(documents(path, kind), 40_000):
            if split_of(doc_id) != split:
                continue
            for lp in lists_in(text):
                md = lp.markdown()
                if md not in seen:
                    seen.add(md)
                    found.append((md, lp, repo, url, lic))
        rng.shuffle(found)
        for md, lp, repo, url, lic in found[:per_shard]:
            pid = hashlib.sha256(md.encode()).hexdigest()[:12]
            rows.append(
                {
                    "id": pid,
                    "domain": "list",
                    "split": split,
                    "text": md,
                    "source": repo,
                    "url": url,
                    "license": lic,
                }
            )
        print(f"{file}: {len(found)} lists, kept {min(per_shard, len(found))}", flush=True)
    out = eval_dir() / "finaltext" / f"passages-lists-{split}.jsonl"
    with out.open("w", encoding="utf-8", newline="\n") as fh:
        for r in rows:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"wrote {len(rows)} -> {out}")


# ---------------------------------------------------------------------------------------------
# Spoken scripts and the prose gold.

UNORDERED = ("plain", "bullets", "also")
ORDERED = ("ordinals", "steps", "then")


def _join(items: list[str]) -> str:
    """'a, b and c': how a run of items is said."""
    items = [it.rstrip(".;,") for it in items]
    return items[0] if len(items) == 1 else ", ".join(items[:-1]) + " and " + items[-1]


def _short(items: list[str]) -> bool:
    return all(len(it.split()) <= 5 for it in items)


def script(lp: ListPassage, variant: str) -> str:
    """What the speaker says for `lp`, enumerating the `variant` way."""
    intro = lp.intro.rstrip(":").strip()
    items = [it.rstrip(".;") for it in lp.items]
    if variant == "plain":
        body = _join(items) if _short(items) else ". ".join(items)
    elif variant == "bullets":
        body = " ".join(f"Bullet point, {it}." for it in items)
    elif variant == "also":
        body = (
            items[0] + ". " + " ".join(f"Also, {it}." for it in items[1:-1]) + f" And {items[-1]}."
        )
    elif variant == "ordinals":
        body = " ".join(f"{_ORDINALS[i].capitalize()}, {it}." for i, it in enumerate(items))
    elif variant == "steps":
        body = " ".join(f"Step {_NUMBERS[i]}, {it}." for i, it in enumerate(items))
    elif variant == "then":
        mid = " ".join(f"Then {it}." for it in items[1:-1])
        body = f"First, {items[0]}. {mid} And finally, {items[-1]}."
    else:
        raise ValueError(variant)
    text = (intro + ": " if intro else "") + body
    return " ".join((text + (" " + lp.outro if lp.outro else "")).split())


def prose_gold(lp: ListPassage, variant: str) -> str:
    """The gold with lists off: what was said, as written prose."""
    intro = lp.intro.rstrip(":").strip()
    items = [it.rstrip(".;") for it in lp.items]
    if variant in ("plain", "bullets", "also") and _short(items):
        body = _join(items) + "."
    elif variant == "ordinals":
        body = " ".join(f"{_ORDINALS[i].capitalize()}, {it}." for i, it in enumerate(items))
    elif variant == "steps":
        body = " ".join(f"Step {i + 1}: {it}." for i, it in enumerate(items))
    elif variant == "then":
        mid = " ".join(f"Then {it}." for it in items[1:-1])
        body = f"First, {items[0]}. {mid} Finally, {items[-1]}."
    else:
        body = " ".join(it[0].upper() + it[1:] + "." for it in items)
    body = " ".join(body.split())
    if intro:
        body = intro + ": " + body
    return body + (" " + lp.outro if lp.outro else "")


def list_script(passage: str, rng: random.Random):
    """A list rendition script for a list passage, or None if `passage` is not one."""
    from eval.finaltext.spoken import Script

    lp = parse(passage)
    if lp is None:
        return None
    variant = rng.choice(ORDERED if lp.ordered else UNORDERED)
    return Script(
        "list", "natural", script(lp, variant), {"variant": variant, "ordered": str(lp.ordered)}
    )


if __name__ == "__main__":
    app()
