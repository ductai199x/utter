"""Gives training pairs a dictionary line, as the product sends one: the user's dictionary holds
some of the terms the speaker meant (names, product names, acronyms) among many unrelated ones,
and each request carries only the terms heard in it (dictionary.relevant_terms, the mirror of
gate::relevant_terms). The model learns to spell a term the dictionary's way when it was said,
to leave a term alone when it was not, and never to type the line.

ft5 never saw the line: given one, it sometimes typed "Dictionary: ..." or bent the right word
toward a term (docs/eval-results.md, 2026-10-11).

Run: uv run python -m eval.finaltext.dictaug IN.jsonl OUT.jsonl --share 0.5
"""

from __future__ import annotations

import json
import random
import re
from collections import Counter
from itertools import pairwise
from pathlib import Path

import typer
from rapidfuzz import process
from rapidfuzz.distance import Levenshtein

from eval.finaltext.dictionary import relevant_terms
from eval.finaltext.spans import _EDGE, spans

_WORD = re.compile(r"[A-Za-z][A-Za-z0-9]*(?:['-][A-Za-z0-9]+)*")
_DICTATED = "Dictated: "


def candidate_terms(text: str, lower: Counter[str]) -> set[str]:
    """What a user would put in a dictionary from this text: acronyms and mixed-case terms
    (spans.py), and names, capitalized mid-sentence and rarely written in lower case."""
    out = {
        s.text
        for s in spans(text)
        if s.kind == "term" and _WORD.fullmatch(s.text) and lower[s.text.lower()] < 3
    }
    for prev, raw in pairwise(text.split()):
        tok = raw.strip(_EDGE)
        if (
            not prev.endswith((".", "!", "?", ":"))
            and len(tok) > 2
            and tok[0].isupper()
            and tok[1:].islower()
            and _WORD.fullmatch(tok)
            and lower[tok.lower()] == 0
        ):
            out.add(tok)
    return out


def _letters(s: str) -> str:
    return "".join(c for c in s.lower() if c.isalnum())


def _near(transcript: str, keys: list[str], by_letters: dict[str, list[str]]) -> list[str]:
    """Pool terms one edit from a word of five letters or more in the transcript."""
    out: list[str] = []
    for w in {_letters(w) for w in transcript.split()}:
        if len(w) < 5:
            continue
        for key, _, _ in process.extract_iter(w, keys, scorer=Levenshtein.distance, score_cutoff=1):
            if key != w:
                out += by_letters[key]
    return out


def main(
    src: Path,
    dst: Path,
    share: float = typer.Option(0.5, help="Share of pairs whose user has a dictionary."),
    seed: int = 0,
    replay: int | None = typer.Option(
        None, help="Write only the pairs given a line, plus this many of the others."
    ),
) -> None:
    rows = [json.loads(line) for line in src.open(encoding="utf-8") if line.strip()]
    golds = [r["completion"][0]["content"] for r in rows]
    lower = Counter(w.strip(_EDGE) for g in golds for w in g.split() if w.islower())
    own = [sorted(candidate_terms(g, lower)) for g in golds]
    pool = sorted({t for ts in own for t in ts})
    by_letters: dict[str, list[str]] = {}
    for t in pool:
        by_letters.setdefault(_letters(t), []).append(t)
    keys = list(by_letters)
    rng = random.Random(seed)
    carried = with_own = with_other = 0
    lined_ids: set[int] = set()
    for r, terms, gold in zip(rows, own, golds, strict=True):
        if rng.random() >= share:
            continue
        user = r["prompt"][1]["content"]
        head, _, transcript = user.rpartition(_DICTATED)
        mine = rng.sample(terms, min(len(terms), rng.randint(1, 3))) if terms else []
        dictionary = mine + rng.sample(pool, rng.randint(10, 150))
        # In real use, a heard term is often not meant: a name close to a word said ("Thorne"
        # for "there"). Random terms rarely are, so some are picked to be.
        if rng.random() < 0.5:
            near = _near(transcript, keys, by_letters)
            near = [t for t in near if t not in gold]
            dictionary += rng.sample(near, min(len(near), 2))
        rng.shuffle(dictionary)
        heard = relevant_terms(transcript, list(dict.fromkeys(dictionary)))
        if not heard:
            continue
        r["prompt"][1]["content"] = (
            head + "Dictionary: " + ", ".join(heard) + "\n" + _DICTATED + transcript
        )
        carried += 1
        lined_ids.add(id(r))
        with_own += any(t in mine for t in heard)
        with_other += any(t not in gold for t in heard)
    if replay is not None:
        # For continuing a model already trained on these pairs: the new lines, plus some of
        # the rest so what it learned is rehearsed, not forgotten.
        lined = [r for r in rows if id(r) in lined_ids]
        plain = [r for r in rows if id(r) not in lined_ids]
        rows = lined + rng.sample(plain, min(replay, len(plain)))
        rng.shuffle(rows)
    with dst.open("w", encoding="utf-8", newline="\n") as fh:
        for r in rows:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(
        f"{carried} of {len(rows)} pairs carry a dictionary line ({with_own} with a term the "
        f"speaker meant, {with_other} with one not meant); pool of {len(pool)} terms -> {dst}"
    )


if __name__ == "__main__":
    typer.run(main)
