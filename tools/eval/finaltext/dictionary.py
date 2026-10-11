"""The dictionary terms a cleanup request carries: a mirror of gate::relevant_terms in
crates/utter-pipeline/src/gate.rs. Only the terms heard as spoken, or as something close
("super base" for "Supabase"), go into the prompt; tests here and there check the same cases.
"""

from __future__ import annotations

import re

_SPLIT = re.compile(r"[^\w'-]+")


def _letters(s: str) -> str:
    return "".join(c for c in s.lower() if c.isalnum())


def _distance(a: str, b: str) -> int:
    prev = list(range(len(b) + 1))
    for i in range(1, len(a) + 1):
        cur = [i] + [0] * len(b)
        for j in range(1, len(b) + 1):
            cur[j] = min(prev[j - 1] + (a[i - 1] != b[j - 1]), prev[j] + 1, cur[j - 1] + 1)
        prev = cur
    return prev[len(b)]


def heard(term: str, words: list[str]) -> bool:
    target = _letters(term)
    if len(target) < 3:
        return False
    for n in (1, 2, 3):
        for i in range(len(words) - n + 1):
            joined = "".join(_letters(w) for w in words[i : i + n])
            if not joined:
                continue
            d = _distance(joined, target)
            if d == 0 or d / max(len(target), len(joined)) <= 0.34:
                return True
    return False


def relevant_terms(text: str, dictionary: list[str]) -> list[str]:
    # Rust splits on anything but alphanumerics, ' and -; \w adds "_", which _letters drops.
    words = [w for w in _SPLIT.split(text.lower().replace("_", " ")) if w]
    return [t for t in dictionary if heard(t, words)]


def test_relevant_terms() -> None:
    d = ["Supabase", "Kubernetes", "Nguyen", "UMAP"]
    assert relevant_terms("we moved the auth to super base and kubernetes", d) == [
        "Supabase",
        "Kubernetes",
    ]
    assert relevant_terms("lunch at noon?", d) == []
