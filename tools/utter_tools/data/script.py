"""A spoken version's script and the checks that keep it faithful.

A script is what the speaker means to say, in written form: the passage itself, or the passage with
material to throw away (fillers, a correction, a retraction) placed by code. The LLM then reads it
aloud; `check` keeps a reading only if the script's plain words appear in it in order and its
numbers are the same digits in the same order (text.numbers), so the gold stays known.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

from utter_tools.text import spoken_digits


@dataclass
class Script:
    kind: str
    style: str
    text: str
    tags: dict[str, str] = field(default_factory=dict)


_ALPHA = re.compile(r"[a-z]+")


def _alpha_words(text: str) -> list[str]:
    """Plain words: tokens of letters only (identifiers, numbers and symbol-bearing tokens are
    read in too many ways to check word by word; the number check covers digits)."""
    words: list[str] = []
    for tok in text.lower().split():
        tok = tok.strip(".,;:!?\"'()[]{}…—–-“”‘’")
        if tok and all(c.isalpha() or c in "'-" for c in tok):
            words.extend(_ALPHA.findall(tok))
    return words


def words_in_order(script: str, spoken: str, min_ratio: float = 0.97) -> bool:
    """The script's plain words appear in the spoken text in order; a word may be read as a
    longer or shorter form of itself ("min" -> "minute")."""
    want = _alpha_words(script)
    if not want:
        return True
    got = _ALPHA.findall(spoken.lower())
    j = found = 0
    for w in want:
        k = j
        end = None
        while k < len(got) and (end := _match_at(got, k, w)) is None:
            k += 1
        if end is not None:
            found += 1
            j = end
    return found / len(want) >= min_ratio


def _match_at(got: list[str], k: int, w: str) -> int | None:
    """Where the match of `w` starting at got[k] ends (exclusive), or None. A word matches
    itself, a longer or shorter form of itself ("min" -> "minute"), or consecutive pieces that
    join up to it: letters read one by one ("gpu" -> "g p u") or a name read as words
    ("waverep" -> "wave rep", "aigvdbench" -> "a i g v d bench")."""
    joined, end = got[k], k + 1
    while joined != w and w.startswith(joined) and end < len(got):
        joined += got[end]
        end += 1
    if joined == w:
        return end
    g = got[k]
    if g.startswith(w) or (w.startswith(g) and len(g) >= 3) or _SPOKEN_AS.get(w) == g:
        return k + 1
    # A misspelled word read as the word meant ("morgage" -> "mortgage") does not match on
    # purpose: the gold keeps the typo, and a pair teaching it would teach misspelling.
    return None


# Abbreviations read as the words they stand for.
_SPOKEN_AS = {"vs": "versus"}


def same_numbers(script: str, spoken: str) -> bool:
    """The same numbers: the same digits in the same order, spoken or written."""
    return spoken_digits(script) == spoken_digits(spoken)


def check(script: str, spoken: str) -> str | None:
    """None if the spoken form is faithful to the script, else why not."""
    if not words_in_order(script, spoken):
        return "words"
    if not same_numbers(script, spoken):
        return "numbers"
    return None
