"""The pieces of a written passage that must come out exactly: numbers, symbols, technical
terms, and bracketed or quoted phrases. Each span has a kind, so the report can say which kind
Utter gets wrong.

- number: any token with a digit ("~240", "72k", "7.5", "3:15", "$25", "2026", "v2.1.3");
- symbol: a token with a symbol people say or imply ("videos/min", "4×", "AT&T", "#hashtag");
- term: an identifier or acronym typed with unusual capitals ("TF32", "AIGVDBench", "GitHub",
  "LTS");
- bracket: a parenthesized phrase, brackets included ("(4× faster)");
- quote: a quoted phrase, quotes included.

A token can be several kinds ("TF32" is a number and a term); it is reported once per kind.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

EDGE = ".,;:!?\"'()[]{}“”‘’…—–"
_SYMBOLS = set("/×~&#@%$€£+°*")
_BRACKET = re.compile(r"\([^()]{1,80}\)")
_QUOTE = re.compile(r"\"[^\"]{1,80}\"|“[^”]{1,80}”")


@dataclass(frozen=True)
class Span:
    text: str
    kind: str


def _is_term(tok: str) -> bool:
    letters = [c for c in tok if c.isalpha()]
    if len(letters) < 2:
        return False
    upper = sum(c.isupper() for c in letters)
    # An acronym (LTS, GPU) or mixed case past the first letter (GitHub, AIGVDBench, iPhone).
    return upper == len(letters) or any(c.isupper() for c in tok[1:] if c.isalpha())


def spans(text: str) -> list[Span]:
    out: list[Span] = []
    for raw in text.split():
        tok = raw.strip(EDGE)
        if not tok:
            continue
        if any(c.isdigit() for c in tok):
            out.append(Span(tok, "number"))
        if any(c in _SYMBOLS for c in tok):
            out.append(Span(tok, "symbol"))
        if _is_term(tok) and not tok.isdigit():
            # "I" and sentence-initial capitals are not terms; single capital letters are not
            # either (letters < 2 above).
            out.append(Span(tok, "term"))
    out += [Span(m.group(), "bracket") for m in _BRACKET.finditer(text)]
    out += [Span(m.group(), "quote") for m in _QUOTE.finditer(text)]
    return out
