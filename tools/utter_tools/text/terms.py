"""What a user would put in a dictionary from a text: acronyms and mixed-case terms (spans' "term"
kind: GitHub, TF32, Kerr-Newman), and names (capitalized mid-sentence and never written in lower
case). Paths and sizes ("/usr/share", "120GB") are not dictionary words, nor everyday words in
capitals ("KEY", "ON": emphasis or SQL), which `lower` (word counts in lower case, over the texts
the terms are drawn from) rules out.
"""

from __future__ import annotations

import re
from collections import Counter
from collections.abc import Iterable
from itertools import pairwise

from utter_tools.text.spans import EDGE, spans

WORD = re.compile(r"[A-Za-z][A-Za-z0-9]*(?:['-][A-Za-z0-9]+)*")


def tokens(text: str) -> Counter[str]:
    """Words as written, with surrounding punctuation stripped."""
    return Counter(t for t in (w.strip(EDGE) for w in text.split()) if t)


def lowercase_counts(texts: Iterable[str]) -> Counter[str]:
    return Counter(t for text in texts for t in tokens(text) if t.islower())


def candidate_terms(text: str, lower: Counter[str], names: bool = True) -> set[str]:
    out = {
        s.text
        for s in spans(text)
        if s.kind == "term" and WORD.fullmatch(s.text) and lower[s.text.lower()] < 3
    }
    if names:
        for prev, raw in pairwise(text.split()):
            tok = raw.strip(EDGE)
            if (
                not prev.endswith((".", "!", "?", ":"))
                and len(tok) > 2
                and tok[0].isupper()
                and tok[1:].islower()
                and WORD.fullmatch(tok)
                and lower[tok.lower()] == 0
            ):
                out.add(tok)
    return out
