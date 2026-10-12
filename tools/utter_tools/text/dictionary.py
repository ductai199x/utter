"""The dictionary terms a cleanup request carries: a mirror of gate::relevant_terms in
crates/utter-pipeline/src/gate.rs. Only the terms heard as spoken, or as something close
("super base" for "Supabase"), go into the prompt; tests here and there check the same cases.
"""

from __future__ import annotations

import re

from rapidfuzz.distance import Levenshtein

_SPLIT = re.compile(r"[^\w'-]+")


def _letters(s: str) -> str:
    return "".join(c for c in s.lower() if c.isalnum())


def _distance(a: str, b: str) -> int:
    # Plain Levenshtein, as gate.rs computes it, in C: training data runs this millions of times.
    return Levenshtein.distance(a, b)


def heard(term: str, words: list[str]) -> bool:
    target = _letters(term)
    if len(target) < 3:
        return False
    for n in (1, 2, 3):
        for i in range(len(words) - n + 1):
            joined = "".join(_letters(w) for w in words[i : i + n])
            if not joined:
                continue
            # The distance is at least the difference in length: skip what cannot match.
            if joined != target and (
                len(target) < 5 or abs(len(joined) - len(target)) > 0.25 * len(target) + 1
            ):
                continue
            d = _distance(joined, target)
            # Short terms are close to too many words ("TOP" to "to"): exact only.
            if d == 0 or (len(target) >= 5 and d / max(len(target), len(joined)) <= 0.25):
                return True
    return False


def relevant_terms(text: str, dictionary: list[str]) -> list[str]:
    # Rust splits on anything but alphanumerics, ' and -; \w adds "_", which _letters drops.
    words = [w for w in _SPLIT.split(text.lower().replace("_", " ")) if w]
    return [t for t in dictionary if heard(t, words)]
