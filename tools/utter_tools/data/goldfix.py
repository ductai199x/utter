"""Minimal fixes to gold passages before they are spoken: clear typos, proper nouns in lowercase,
chat shorthand (the proofreading pass), then grammar errors (the grammar pass: Utter fixes grammar
by default, docs/cleanup.md rule 8, so a gold with a grammar error would teach and score
keeping it). The writer's own choices (punctuation style, tone, fragments) are kept.

The LLM proposes fixes as data (the exact wrong text and its replacement); code applies each only
where it matches exactly once (a grammar fix only if ERRANT confirms it is grammar), and drops the
passage if the fixes would change more than MAX_CHANGE of it, or if it is not dictatable prose.
"""

from __future__ import annotations

import json
import re
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass

from rapidfuzz.distance import Levenshtein

from utter_tools import common
from utter_tools.data.grammar import annotator
from utter_tools.llm import Chat

SYSTEM = common.prompt("goldfix")

SCHEMA = {
    "type": "object",
    "properties": {
        "not_prose": {"type": "boolean"},
        "fixes": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "kind": {"enum": ["typo", "capitalization", "shorthand"]},
                    "wrong": {"type": "string"},
                    "right": {"type": "string"},
                },
                "required": ["kind", "wrong", "right"],
            },
        },
    },
    "required": ["not_prose", "fixes"],
}

# Fixes may change at most this share of a passage's characters.
MAX_CHANGE = 0.12

# The grammar pass, run on passages the proofreading pass has fixed.
GRAMMAR_SYSTEM = common.prompt("goldfix-grammar")

GRAMMAR_SCHEMA = {
    "type": "object",
    "properties": {
        "fixes": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "kind": {"enum": ["grammar", "apostrophe", "capitalization"]},
                    "wrong": {"type": "string"},
                    "right": {"type": "string"},
                },
                "required": ["kind", "wrong", "right"],
            },
        },
    },
    "required": ["fixes"],
}

# ERRANT types a grammar-pass fix may make; a fix whose edits include anything else (a
# different noun, a new idea) is not grammar and is not applied.
GRAMMAR_TYPES = {
    "DET", "PREP", "PRON", "VERB:TENSE", "VERB:SVA", "VERB:FORM", "VERB:INFL", "VERB",
    "NOUN:NUM", "NOUN:INFL", "NOUN:POSS", "ADJ:FORM", "WO", "PART", "CONJ", "MORPH", "ORTH",
    "CONTR", "PUNCT",
}  # fmt: skip


def apply_grammar(text: str, fixes: list[dict]) -> Outcome:
    """Applies the grammar fixes ERRANT confirms are grammar, each where it matches once."""
    a = annotator()
    applied = []
    changed = 0
    for f in fixes:
        wrong, right = f["wrong"], f["right"]
        where = list(re.finditer(rf"(?<!\w){re.escape(wrong)}(?!\w)", text)) if wrong else []
        if wrong == right or len(where) != 1:
            continue
        m = where[0]
        candidate = text[: m.start()] + right + text[m.end() :]
        edits = a.annotate(a.parse(text, tokenise=True), a.parse(candidate, tokenise=True))
        types = {e.type.split(":", 1)[1] for e in edits if e.type not in ("noop", "UNK")}
        if not types or not types <= GRAMMAR_TYPES:
            continue
        text = candidate
        changed += Levenshtein.distance(wrong, right)
        applied.append(f)
    if changed > MAX_CHANGE * len(text):
        return Outcome(None, applied, "too many fixes")
    return Outcome(text, applied)


@dataclass
class Outcome:
    text: str | None  # the fixed passage, or None to drop it
    applied: list[dict]
    why_dropped: str = ""


def apply(text: str, not_prose: bool, fixes: list[dict]) -> Outcome:
    if not_prose:
        return Outcome(None, [], "not prose")
    applied = []
    changed = 0
    for f in fixes:
        wrong, right = f["wrong"], f["right"]
        # Whole words only: "sea" must not match inside "research".
        where = list(re.finditer(rf"(?<!\w){re.escape(wrong)}(?!\w)", text)) if wrong else []
        if wrong == right or len(where) != 1:
            continue  # vague or ambiguous: not applied
        m = where[0]
        text = text[: m.start()] + right + text[m.end() :]
        # Characters actually changed: "ubuntu" -> "Ubuntu" is one.
        changed += Levenshtein.distance(wrong, right)
        applied.append(f)
    if changed > MAX_CHANGE * len(text):
        return Outcome(None, applied, "too many fixes")
    return Outcome(text, applied)


def fix(
    passages: list[dict], llm: Chat, grammar: bool, workers: int = 8
) -> tuple[list[dict], dict[str, int]]:
    """One pass over `passages` (proofreading, or with `grammar` the grammar pass): the passages
    kept, fixed where needed, and how many were dropped for each reason."""
    system, schema = (GRAMMAR_SYSTEM, GRAMMAR_SCHEMA) if grammar else (SYSTEM, SCHEMA)

    def one(p: dict) -> tuple[dict, Outcome]:
        out = llm.chat(
            [{"role": "system", "content": system}, {"role": "user", "content": p["final_text"]}],
            max_tokens=1024,
            temperature=0.0,
            json_schema=schema,
        )
        try:
            reply = json.loads(out)
        except json.JSONDecodeError:
            # A reply cut off at the token limit: a passage needing that many fixes is not one to
            # keep anyway.
            return p, Outcome(None, [], "unreadable reply")
        if grammar:
            return p, apply_grammar(p["final_text"], reply["fixes"])
        return p, apply(p["final_text"], reply["not_prose"], reply["fixes"])

    kept: list[dict] = []
    dropped: dict[str, int] = {}
    with ThreadPoolExecutor(workers) as pool:
        for p, o in pool.map(one, passages):
            if o.text is None:
                dropped[o.why_dropped] = dropped.get(o.why_dropped, 0) + 1
            else:
                kept.append({**p, "final_text": o.text})
    return kept, dropped
