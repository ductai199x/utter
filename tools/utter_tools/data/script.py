"""A spoken version's script and the checks that keep it faithful.

A script is what the speaker means to say, in written form: the passage itself, or the passage with
material to throw away (fillers, a correction, a retraction) placed by code. The LLM then reads it
aloud; `check` keeps a reading only if the script's plain words appear in it in order, it adds
next to nothing (no negation, ever), and its numbers are the same digits in the same order
(text.spoken_digits), so the gold stays known.
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
    """The script's plain words appear in the spoken text in order: a word may be read as a
    longer or shorter form of itself ("min" -> "minute"), never as its negation."""
    want = _alpha_words(script)
    found, _ = _align(want, _ALPHA.findall(spoken.lower()))
    return not want or found / len(want) >= min_ratio


def added_words(script: str, spoken: str) -> list[str]:
    """What a reading says that the script does not, beyond what a reading may add (numbers and
    punctuation said as words, fillers): a few slips are tolerated by `check`, a negation never."""
    _, extra = _align(_alpha_words(script), _ALPHA.findall(spoken.lower()))
    return [w for w in extra if w not in _MAY_ADD]


def _align(want: list[str], got: list[str]) -> tuple[int, list[str]]:
    """Matches `want` in `got` in order: how many matched, and the words of `got` left over."""
    j = found = 0
    extra: list[str] = []
    for w in want:
        k = j
        end = None
        while k < len(got) and (end := _match_at(got, k, w)) is None:
            k += 1
        if end is not None:
            found += 1
            extra += got[j:k]
            j = end
    return found, extra + got[j:]


# Words a reading adds that it should not: at most this share of the script's words (one at
# least, for a slip).
EXTRA_SHARE = 0.03

# What a reading may add, by why.
_MAY_ADD = frozenset(
    word
    for group in (
        # Numbers said as words (the number check compares the digits).
        (
            "zero oh one two three four five six seven eight nine ten eleven twelve thirteen"
            " fourteen fifteen sixteen seventeen eighteen nineteen twenty thirty forty fifty sixty"
            " seventy eighty ninety hundred thousand million billion point half quarter percent"
        ),
        # Punctuation said as words (the explicit style).
        (
            "comma period dot full stop question exclamation mark colon semicolon dash hyphen"
            " quote unquote open close paren parenthesis bracket new line paragraph bullet slash"
        ),
        # Fillers.
        "um uh er ah hmm like so well okay",
        # The pieces contractions split into ("don't" -> "don", "t").
        "t s re ve ll d m",
    )
    for word in group.split()
)
# Words that flip what is said: a reading never adds one.
_NEGATIONS = frozenset({"not", "never", "no", "nt"})
# Verbs with a negated form that reads as a word of its own ("don" of don't, "cannot").
_NEGATABLE = frozenset(
    {
        "do",
        "does",
        "did",
        "is",
        "are",
        "was",
        "were",
        "has",
        "have",
        "had",
        "can",
        "could",
        "should",
        "would",
        "might",
        "must",
        "need",
    }
)


def _negates(word: str, other: str) -> bool:
    """`other` is `word` negated: "don" (of don't) for "do", "cannot" for "can"."""
    return word in _NEGATABLE and other in (word + "n", word + "nt", word + "not")


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
    # A negation is not a form of the word, either way round.
    if _negates(w, g) or _negates(g, w):
        return None
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
    added = added_words(script, spoken)
    if any(w in _NEGATIONS for w in added) or len(added) > max(
        1, round(EXTRA_SHARE * len(_alpha_words(script)))
    ):
        return "added"
    if not same_numbers(script, spoken):
        return "numbers"
    return None
