"""Spoken forms of written passages: how a person would dictate each one.

Every passage is written text someone actually typed: the gold result. A spoken form is made in
two steps, so its gold is known by construction:

1. A *script* is built in written form. For most kinds it is the passage itself. For the kinds
   with something to throw away (disfluencies, corrections, retraction commands), code inserts
   the extra material into the passage: fillers and repeats at random word boundaries, and
   pieces the LLM proposes as structured data (a phrase quoted exactly from the passage, a
   plausible wrong alternative, a cue). Code places them, so a correction always ends on the
   passage's own words.
2. The LLM reads the script aloud as a dictating person would: numbers, units, symbols and
   identifiers in words, punctuation either unspoken (natural) or said by name (explicit).

A spoken form is kept only if it passes `check` against its script: the script's plain words
appear in order, and the numbers (spoken words read back as digits, text.numbers) are
the same digits in the same order (script.check). The gold for every kind is the passage.

Renditions are speech rows without audio (schema.py): `id` is `<passage>-<category>`,
`transcript` what is said, `annotations` how (the cue, the scope, the fillers...).
"""

from __future__ import annotations

import random
import re
from collections.abc import Iterable, Iterator
from concurrent.futures import ThreadPoolExecutor

from utter_tools import common, schema
from utter_tools.data.amend import amend_script
from utter_tools.data.grammar import grammar_script
from utter_tools.data.lists import list_script
from utter_tools.data.script import Script, check
from utter_tools.llm import Chat

KINDS = ("natural", "explicit", "disfluent", "correction", "command")
# Also available: "restate", a correction of a whole sentence (a wrong sentence, a cue such as
# "scratch that", the right sentence); "grammar", the passage said with grammar mistakes
# (grammar.py); "list", a list passage enumerated aloud (lists.py); and "amend", a detail said
# wrong and fixed later by reference (amend.py).
EXTRA_KINDS = ("restate", "grammar", "list", "amend")

# ---------------------------------------------------------------------------------------------
# Step 2: reading a script aloud.

_READ = common.prompt("read")

_STYLE = {style: common.prompt(f"read-{style}") for style in ("natural", "explicit")}


def read_prompt(style: str, script: str) -> list[dict[str, str]]:
    return [
        {"role": "system", "content": _READ + "\n\n" + _STYLE[style]},
        {"role": "user", "content": f"Text:\n{script}"},
    ]


# ---------------------------------------------------------------------------------------------
# Step 1: scripts.

_FILLERS = ("um", "uh", "um", "uh", "like", "you know", "so", "kind of")
# Cues a correction can use; "" is a bare restatement.
CUES = ("no", "no wait", "sorry", "I mean", "actually", "or rather", "wait", "")
SENTENCE_CUES = ("scratch that", "let me rephrase", "sorry, let me say that again", "actually, no")
COMMANDS = (
    "scratch that",
    "delete that",
    "never mind, let's start over",
    "let's try that from the beginning",
    "start over",
)
# Commands that retract the last sentence; the others retract everything said so far.
SENTENCE_COMMANDS = ("scratch that", "delete that")

_PROPOSE_CORRECTION = common.prompt("propose-correction")

# How often each correction scope is asked for. Left to itself the LLM never chose "sentence".
SCOPES = ("word", "word", "word", "phrase", "phrase", "sentence", "sentence")

_PROPOSE_FALSE_START = common.prompt("propose-false-start")

_PROPOSE_RESTART = common.prompt("propose-restart")

_CORRECTION_SCHEMA = {
    "type": "object",
    "properties": {
        "scope": {"enum": ["word", "phrase", "sentence"]},
        "anchor": {"type": "string"},
        "wrong": {"type": "string"},
    },
    "required": ["scope", "anchor", "wrong"],
}
_FALSE_START_SCHEMA = {
    "type": "object",
    "properties": {"false_start": {"type": "string"}},
    "required": ["false_start"],
}
_RESTART_SCHEMA = {
    "type": "object",
    "properties": {"anchor": {"type": "string"}, "abandoned": {"type": "string"}},
    "required": ["anchor", "abandoned"],
}


def _boundaries(passage: str) -> list[int]:
    """Offsets of word starts (not the first word), outside brackets and quotes is not
    required: a filler can fall anywhere a speaker can hesitate, which is anywhere."""
    return [m.start() for m in re.finditer(r"(?<=\s)\S", passage)]


def _insert(passage: str, at: int, piece: str) -> str:
    return passage[:at] + piece + passage[at:]


def _proposal(
    llm: Chat, system: str, passage: str, schema_: dict, seed: int, note: str = ""
) -> dict | None:
    user = f"Passage:\n{passage}" + (f"\n\n{note}" if note else "")
    return llm.ask(system, user, schema_, seed)


def disfluent_script(passage: str, rng: random.Random, llm: Chat, seed: int) -> Script | None:
    if len(_boundaries(passage)) < 4:
        return None
    text = passage
    tags: dict[str, str] = {}
    # The false start first, while the passage's phrases are still contiguous; then fillers.
    p = _proposal(llm, _PROPOSE_RESTART, passage, _RESTART_SCHEMA, seed)
    if p:
        anchor, abandoned = p.get("anchor", "").strip(), p.get("abandoned", "").strip()
        if (
            anchor
            and abandoned
            and passage.count(anchor) == 1
            and abandoned.lower() not in anchor.lower()
        ):
            text = _insert(text, text.index(anchor), abandoned + ", ")
            tags["false_start"] = "1"
    starts = _boundaries(text)
    n = max(2, min(6, len(starts) // 8))
    # Insert from the end so earlier offsets stay valid.
    picks = sorted(rng.sample(starts, min(n, len(starts))), reverse=True)
    repeats = 0
    for at in picks:
        if rng.random() < 0.35:
            word = re.match(r"\S+", text[at:])
            if word and word.group().isalpha() and len(word.group()) <= 6:
                text = _insert(text, at, word.group() + " ")
                repeats += 1
                continue
        text = _insert(text, at, rng.choice(_FILLERS) + ", ")
    tags["fillers"] = str(len(picks) - repeats)
    tags["repeats"] = str(repeats)
    return Script("disfluent", "natural", text, tags)


def correction_script(
    passage: str, rng: random.Random, llm: Chat, seed: int, force_scope: str | None = None
) -> Script | None:
    want = force_scope or rng.choice(SCOPES)
    p = _proposal(
        llm, _PROPOSE_CORRECTION, passage, _CORRECTION_SCHEMA, seed, note=f"Scope: {want}"
    )
    if not p:
        return None
    scope, anchor, wrong = (
        p.get("scope", ""),
        p.get("anchor", "").strip(),
        p.get("wrong", "").strip(),
    )
    if (
        scope != want
        or not anchor
        or not wrong
        or passage.count(anchor) != 1
        or wrong.lower() == anchor.lower()
    ):
        return None
    idx = passage.index(anchor)
    # A sentence-scope anchor must be a whole sentence of the passage.
    if scope == "sentence" and not (
        anchor[-1] in ".!?" and (idx == 0 or passage[:idx].rstrip()[-1:] in ".!?")
    ):
        return None
    if scope == "sentence":
        cue = rng.choice(SENTENCE_CUES)
        wrong = wrong.rstrip(".!?") + "."
        piece = f"{wrong} {cue[0].upper() + cue[1:]}. "
    else:
        cue = rng.choice(CUES)
        piece = f"{wrong}, {cue}, " if cue else f"{wrong}, "
    return Script(
        "correction",
        "natural",
        _insert(passage, idx, piece),
        {"scope": scope, "cue": cue or "none"},
    )


def command_script(passage: str, rng: random.Random, llm: Chat, seed: int) -> Script | None:
    p = _proposal(llm, _PROPOSE_FALSE_START, passage, _FALSE_START_SCHEMA, seed)
    start = p.get("false_start", "").strip() if p else ""
    if not start:
        return None
    command = rng.choice(COMMANDS)
    if command in SENTENCE_COMMANDS:
        # These retract the sentence before them, not everything (docs/cleanup.md): a
        # longer false start would be partly kept, and the passage would not be the gold.
        start = re.split(r"(?<=[.!?])\s+", start)[-1]
    start = start.rstrip(".!?") + "."
    return Script(
        "command",
        "natural",
        f"{start} {command[0].upper() + command[1:]}. {passage}",
        {"command": command},
    )


def make_script(kind: str, passage: str, rng: random.Random, llm: Chat, seed: int) -> Script | None:
    if kind in ("natural", "explicit"):
        return Script(kind, kind, passage)
    if kind == "disfluent":
        return disfluent_script(passage, rng, llm, seed)
    if kind == "restate":
        s = correction_script(passage, rng, llm, seed, force_scope="sentence")
        return Script("restate", s.style, s.text, s.tags) if s else None
    if kind == "correction":
        return correction_script(passage, rng, llm, seed)
    if kind == "command":
        return command_script(passage, rng, llm, seed)
    if kind == "grammar":
        return grammar_script(passage, rng, llm, seed)
    if kind == "list":
        return list_script(passage, rng)
    if kind == "amend":
        return amend_script(passage, rng, llm, seed)
    raise ValueError(kind)


_SENTENCE_ENDS = re.compile(r"[.!?](?:\s|$)")


def kinds_for(passage: dict, kinds: Iterable[str], per_passage: int) -> list[str]:
    """The kinds a passage is rendered in. A restatement needs a sentence to correct and one to
    keep, so only passages of two or more sentences get one. With `per_passage`, each passage gets
    that many of its kinds, chosen by its id: a model trained on six renditions of one passage
    learns the passage by heart."""
    mine = [
        k
        for k in kinds
        if k != "restate" or len(_SENTENCE_ENDS.findall(passage["final_text"])) >= 2
    ]
    if per_passage:
        mine = random.Random(f"kinds:{passage['id']}").sample(mine, min(per_passage, len(mine)))
    return mine


def rendition(passage: dict, script: Script, spoken: str) -> dict:
    """A rendition row: a speech row without audio (schema.SPEECH_SCHEMA's text fields)."""
    return {
        "id": f"{passage['id']}-{script.kind}",
        "category": script.kind,
        "domain": passage["domain"],
        "transcript": spoken,
        "final_text": passage["final_text"],
        "annotations": [f"{k}={v}" for k, v in script.tags.items()],
        **{k: passage[k] for k in schema.PROVENANCE},
    }


def render(
    llm: Chat,
    passages: list[dict],
    kinds: Iterable[str],
    done: set[str],
    seed: int,
    per_passage: int = 0,
    workers: int = 4,
    tries: int = 3,
) -> Iterator[tuple[str, dict | None, list[str]]]:
    """Yields (rendition id, row or None, rejections) for every passage and kind whose rendition
    is not in `done`, passage by passage: a run stopped partway leaves every kind equally
    covered, not the first kinds only."""
    jobs = [
        (p, k)
        for p in passages
        for k in kinds_for(p, kinds, per_passage)
        if f"{p['id']}-{k}" not in done
    ]

    def one(job: tuple[dict, str]) -> tuple[str, dict | None, list[str]]:
        p, kind = job
        rid = f"{p['id']}-{kind}"
        rejections: list[str] = []
        for attempt in range(tries):
            # One generator per job and attempt: reproducible whatever order threads run in.
            rng = random.Random(f"{seed}:{p['id']}:{kind}:{attempt}")
            script = make_script(kind, p["final_text"], rng, llm, seed=attempt)
            if script is None:
                rejections.append("script")
                continue
            out = llm.chat(
                read_prompt(script.style, script.text),
                max_tokens=1024,
                temperature=0.8,
                seed=attempt,
            )
            spoken = " ".join(out.strip().split())
            why = check(script.text, spoken)
            if why is None:
                return rid, rendition(p, script, spoken), rejections
            rejections.append(why)
        return rid, None, rejections

    with ThreadPoolExecutor(workers) as pool:
        yield from pool.map(one, jobs)
