"""Minimal fixes to gold passages: clear typos, proper nouns in lowercase, chat shorthand. The
writer's own choices (punctuation style, tone, fragments) are kept.

Excluding every passage the screen flags throws away good text on the screen's noisy opinions
and keeps the errors it misses; fixing is better. The LLM proposes fixes as data (the exact
wrong text and its replacement); code applies each only where it matches exactly once, and
drops the passage if the fixes would change more than a small share of it, or if it is not
dictatable prose at all.

For a split already rendered (eval v1), only fixes that do not change the spoken words apply
(capitalization); passages needing others are dropped, since their spoken forms were made from
the unfixed text.

Run: uv run python -m eval.finaltext.goldfix --split train
     uv run python -m eval.finaltext.goldfix --split eval --spoken-safe
"""

from __future__ import annotations

import json
import re
import shutil
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass

import typer

from eval.finaltext.score import edit_distance
from eval.paths import eval_dir

SYSTEM = """You proofread sentences that will be the answer key for a voice-typing app, which must
type exactly what a careful, fluent writer would. Propose fixes ONLY for clear errors:

- typo: a misspelled word ("hasle" -> "hassle", "recieve" -> "receive");
- capitalization: a proper noun, product, language or name written in lowercase ("ubuntu" ->
  "Ubuntu", "ruby" -> "Ruby"), a sentence starting in lowercase, or a word in capitals for
  emphasis ("you WANT this" -> "you want this"). Acronyms (GPU, LTS) and names that are
  conventionally lowercase (feh, npm, iPhone) are not errors;
- shorthand: chat abbreviations a speaker would say as words ("d/l" -> "download", "u" -> "you",
  "pls" -> "please", "thx" -> "thanks").

Do not touch anything else: no rewording, no grammar changes, no punctuation changes, no
style changes. Informal tone, contractions and fragments are fine. Most sentences need nothing.

If the text is not prose someone would dictate (a citation, a reference list, a table row, a
file listing, code, a non-English passage), set "not_prose" to true.

Return JSON: {"not_prose": bool, "fixes": [{"kind": ..., "wrong": exact text from the sentence,
"right": replacement}]}."""

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

# The grammar pass (--grammar), run on passages already fixed above: Utter fixes grammar by
# default (docs/cleanup-spec.md, rule 8), so an answer key with a grammar error would teach
# and score keeping it.
GRAMMAR_SYSTEM = """You proofread sentences that will be the answer key for a voice-typing app
that writes correct, natural English. Propose fixes ONLY for grammar errors a careful editor
would correct:

- grammar: subject-verb agreement, tense, verb forms, articles, prepositions, noun number,
  pronoun case, comparatives, word order, a missing word the grammar needs;
- apostrophe: a missing or wrong apostrophe ("dont" -> "don't", "its" -> "it's", "users"
  -> "user's" when possessive);
- capitalization: a sentence or the pronoun "I" in lowercase ("i think" -> "I think").

Keep everything else exactly: wording, tone, contractions, informal words ("gonna", "yeah"),
fragments that read fine in writing ("Works for me."), punctuation style, British or American
spelling. Do not rephrase correct sentences. Most sentences need nothing.

Each fix is the shortest exact span from the sentence ("wrong") and its replacement ("right").
Return JSON: {"fixes": [{"kind": ..., "wrong": ..., "right": ...}]}."""

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
    from eval.finaltext.grammar import _annotator

    a = _annotator()
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
        changed += edit_distance(wrong, right)
        applied.append(f)
    if changed > MAX_CHANGE * len(text):
        return Outcome(None, applied, "too many fixes")
    return Outcome(text, applied)


@dataclass
class Outcome:
    text: str | None  # the fixed passage, or None to drop it
    applied: list[dict]
    why_dropped: str = ""


def apply(text: str, not_prose: bool, fixes: list[dict], spoken_safe: bool) -> Outcome:
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
        if spoken_safe and (f["kind"] != "capitalization" or wrong.lower() != right.lower()):
            # The spoken form was made from the unfixed text: only case may change.
            return Outcome(None, [], f"needs a {f['kind']} fix")
        m = where[0]
        text = text[: m.start()] + right + text[m.end() :]
        # Characters actually changed: "ubuntu" -> "Ubuntu" is one.
        changed += edit_distance(wrong, right)
        applied.append(f)
    if changed > MAX_CHANGE * len(text):
        return Outcome(None, applied, "too many fixes")
    return Outcome(text, applied)


def main(
    split: str = "train",
    spoken_safe: bool = typer.Option(False, help="Only fixes that keep the spoken words (case)."),
    grammar: bool = typer.Option(
        False, help="The grammar pass, on passages already fixed (kept once as .pre-grammar)."
    ),
    model: str = "gemma4-26b-a4b-qat",
    slots: int = 8,
    llm_url: str = typer.Option(
        "", help="An OpenAI-style server to use instead of a local llama-server (e.g. vLLM)."
    ),
) -> None:
    from eval.finaltext.llm import Remote, Server, model_path

    root = eval_dir() / "finaltext"
    path = root / f"passages-{split}.jsonl"
    # Each pass reads the file it started from, kept once, so a rerun redoes it, not stacks it.
    source = root / f"passages-{split}.{'pre-grammar' if grammar else 'raw'}.jsonl"
    if not source.exists():
        shutil.copy(path, source)
    passages = [
        json.loads(line) for line in source.read_text(encoding="utf-8").splitlines() if line
    ]
    system, schema = (GRAMMAR_SYSTEM, GRAMMAR_SCHEMA) if grammar else (SYSTEM, SCHEMA)

    server = Remote(llm_url) if llm_url else Server(model_path(model), slots=slots)
    with server as llm:

        def one(p: dict) -> tuple[dict, Outcome]:
            out = llm.chat(
                [{"role": "system", "content": system}, {"role": "user", "content": p["text"]}],
                max_tokens=1024,
                temperature=0.0,
                json_schema=schema,
            )
            try:
                reply = json.loads(out)
            except json.JSONDecodeError:
                # A reply cut off at the token limit: a passage needing that many fixes is not
                # one to keep anyway.
                return p, Outcome(None, [], "unreadable reply")
            if grammar:
                return p, apply_grammar(p["text"], reply["fixes"])
            return p, apply(p["text"], reply["not_prose"], reply["fixes"], spoken_safe)

        kept: list[dict] = []
        dropped: dict[str, int] = {}
        fixed = 0
        with ThreadPoolExecutor(slots) as pool:
            for p, o in pool.map(one, passages):
                if o.text is None:
                    dropped[o.why_dropped] = dropped.get(o.why_dropped, 0) + 1
                    continue
                if o.applied and grammar:
                    fixed += 1
                    p = {
                        **p,
                        "text": o.text,
                        "before_grammar": p["text"],
                        "grammar_fixes": o.applied,
                    }
                elif o.applied:
                    fixed += 1
                    p = {**p, "text": o.text, "original": p["text"], "fixes": o.applied}
                kept.append(p)
    with path.open("w", encoding="utf-8") as fh:
        for p in kept:
            fh.write(json.dumps(p, ensure_ascii=False) + "\n")
    print(f"{len(kept)} kept ({fixed} fixed), dropped {dropped} -> {path}")


if __name__ == "__main__":
    typer.run(main)
