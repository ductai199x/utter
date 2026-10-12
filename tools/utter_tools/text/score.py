"""Scoring a dictation's final text against the written text it should have produced.

A written text is read as words, each followed by its *gap*: everything between it and the
next word (spaces, punctuation, brackets, quotes, dashes, line breaks). The words of the output
are aligned to the words of the reference; then:

- marks: the gaps of aligned words are compared mark by mark, so every punctuation mark,
  bracket, quote, dash and line break gets precision, recall and F1, including the ones
  nobody says;
- casing: aligned words with the same letters must have the same capitals;
- faithfulness: word errors after both texts are normalized (Whisper's English normalizer), so
  "~240" against "about 240" costs one word, not a formatting mismatch;
- exact: the output needs no edit at all (spacing and quote style aside);
- chars to fix: character edit distance, relative to the reference length;
- spans: tagged pieces of the reference ("(4× faster)", "TF32") must appear verbatim.
"""

from __future__ import annotations

import re
import unicodedata
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass, field

# Marks that can stand between words. A run of them at the start or end of a
# whitespace-delimited chunk belongs to the gap; inside a chunk ("don't", "3.5", "a/b",
# "protocol-eligible") it belongs to the word.
_MARK_CHARS = set(".,?!:;\"'()[]{}…—–-“”‘’«»")
# Characters that normalize to one class when compared: curly and straight quotes are the
# same mark; the dashes are one "dash" (spacing and length are style).
_MARK_CLASS = {
    "“": '"',
    "”": '"',
    "«": '"',
    "»": '"',
    "‘": "'",
    "’": "'",
    "—": "—",
    "–": "—",
    "-": "—",
}
_WS = re.compile(r"[ \t]+")


@dataclass(frozen=True)
class Word:
    surface: str
    # Lowercase letters and digits only: what alignment compares.
    core: str


@dataclass
class Parsed:
    words: list[Word]
    # gaps[i] is the text before words[i]; gaps[len(words)] is the text after the last word.
    gaps: list[str]


def parse(text: str) -> Parsed:
    words: list[Word] = []
    gaps: list[str] = []
    pending = ""
    for m in re.finditer(r"\S+|\s+", text):
        chunk = m.group()
        if chunk.isspace():
            pending += chunk
            continue
        start = 0
        while start < len(chunk) and chunk[start] in _MARK_CHARS:
            start += 1
        end = len(chunk)
        while end > start and chunk[end - 1] in _MARK_CHARS:
            end -= 1
        if start == end:  # marks only, e.g. a lone "—"
            pending += chunk
            continue
        pending += chunk[:start]
        gaps.append(pending)
        surface = chunk[start:end]
        words.append(Word(surface, _core(surface)))
        pending = chunk[end:]
    gaps.append(pending)
    return Parsed(words, gaps)


def _core(surface: str) -> str:
    folded = "".join(c for c in unicodedata.normalize("NFKC", surface).lower() if c.isalnum())
    # A word of symbols only ("×", "~") keeps its symbols, so it can still align.
    return folded or surface


def marks(gap: str) -> Counter[str]:
    """The marks in a gap, by class; a line break counts as "\\n" and a blank line as "\\n\\n"."""
    out: Counter[str] = Counter()
    for c in gap:
        if c in _MARK_CHARS:
            out[_MARK_CLASS.get(c, c)] += 1
    newlines = gap.count("\n")
    if newlines >= 2:
        out["\n\n"] += 1
    elif newlines == 1:
        out["\n"] += 1
    return out


def align(ref: list[str], hyp: list[str]) -> list[tuple[int | None, int | None]]:
    """Word alignment by edit distance: pairs of (ref index, hyp index); None on the side of an
    insertion or deletion. Matches are preferred over substitutions on ties."""
    n, m = len(ref), len(hyp)
    cost = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(1, n + 1):
        cost[i][0] = i
    for j in range(1, m + 1):
        cost[0][j] = j
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            sub = cost[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1])
            cost[i][j] = min(sub, cost[i - 1][j] + 1, cost[i][j - 1] + 1)
    pairs: list[tuple[int | None, int | None]] = []
    i, j = n, m
    while i > 0 or j > 0:
        if i > 0 and j > 0 and cost[i][j] == cost[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1]):
            pairs.append((i - 1, j - 1))
            i, j = i - 1, j - 1
        elif i > 0 and cost[i][j] == cost[i - 1][j] + 1:
            pairs.append((i - 1, None))
            i -= 1
        else:
            pairs.append((None, j - 1))
            j -= 1
    pairs.reverse()
    return pairs


def _canonical(text: str) -> str:
    """Spacing and quote style removed, for the exact-match test."""
    text = "".join(_MARK_CLASS.get(c, c) if c in "“”«»‘’" else c for c in text)
    lines = [_WS.sub(" ", line).strip() for line in text.strip().splitlines()]
    return "\n".join(lines)


def edit_distance(a: str, b: str) -> int:
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


@dataclass
class Score:
    exact: bool
    chars_to_fix: float
    # Faithfulness on normalized words.
    ref_words: int
    substitutions: int
    deletions: int
    insertions: int
    # Per mark class: true positives, false positives, false negatives.
    marks: dict[str, list[int]] = field(default_factory=dict)
    casing_checked: int = 0
    casing_wrong: int = 0
    spans_total: int = 0
    spans_hit: int = 0
    missed_spans: list[str] = field(default_factory=list)


_BRACKETS = str.maketrans("()[]{}", "      ")


def _english(text: str) -> str:
    """Whisper's English normalizer, keeping bracketed words: it deletes "(...)" and "[...]"
    (meant for annotations like "[noise]"), which here are words the speaker meant."""
    from utter_tools.text.normalize import normalize

    return normalize(text.translate(_BRACKETS), "en")


def score(
    reference: str,
    hypothesis: str,
    spans: list[str] | None = None,
    normalize: Callable[[str], str] = _english,
) -> Score:
    """Scores one output. `normalize` maps text to normalized text for faithfulness (Whisper's
    English normalizer unless given)."""
    ref_c, hyp_c = _canonical(reference), _canonical(hypothesis)
    exact = ref_c == hyp_c
    chars = edit_distance(ref_c, hyp_c) / max(1, len(ref_c))

    nr, nh = normalize(reference).split(), normalize(hypothesis).split()
    s = d = ins = 0
    for a, b in align(nr, nh):
        if a is None:
            ins += 1
        elif b is None:
            d += 1
        elif nr[a] != nh[b]:
            s += 1

    pr, ph = parse(reference), parse(hypothesis)
    result = Score(exact, chars, len(nr), s, d, ins)
    tally: dict[str, list[int]] = {}

    def count(gold: Counter[str], got: Counter[str]) -> None:
        for k in gold.keys() | got.keys():
            t = tally.setdefault(k, [0, 0, 0])
            hit = min(gold[k], got[k])
            t[0] += hit
            t[1] += got[k] - hit
            t[2] += gold[k] - hit

    pairs = align([w.core for w in pr.words], [w.core for w in ph.words])
    # The gap before each aligned word (and the one after the last) is compared; the gaps
    # around inserted or deleted words merge into their neighbours' so a dropped word does not
    # also count as dropped punctuation.
    gold_gap, got_gap = pr.gaps[0], ph.gaps[0]
    for a, b in pairs:
        if a is not None and b is not None:
            count(marks(gold_gap), marks(got_gap))
            wr, wh = pr.words[a], ph.words[b]
            if wr.core == wh.core:
                result.casing_checked += 1
                result.casing_wrong += wr.surface != wh.surface
            gold_gap, got_gap = pr.gaps[a + 1], ph.gaps[b + 1]
        elif a is not None:
            gold_gap += pr.gaps[a + 1]
        else:
            assert b is not None
            got_gap += ph.gaps[b + 1]
    count(marks(gold_gap), marks(got_gap))
    result.marks = tally

    hyp_q = _canonical(hypothesis)
    for span in spans or []:
        result.spans_total += 1
        if _canonical(span) in hyp_q:
            result.spans_hit += 1
        else:
            result.missed_spans.append(span)
    return result


def prf(t: list[int]) -> tuple[float, float, float]:
    tp, fp, fn = t
    p = tp / (tp + fp) if tp + fp else 1.0
    r = tp / (tp + fn) if tp + fn else 1.0
    f = 2 * p * r / (p + r) if p + r else 0.0
    return p, r, f
