"""Far-back edits for the "amend" rendition kind (docs/cleanup.md, recipe R4): the
speaker says a detail wrong and fixes it later by reference, a sentence or more on, in the ways
people do: "actually, change the eggs to three boxes", "take the umbrella off the list", "oh,
and add bread after milk", "make the second one oat milk". The gold is the passage as meant.

The LLM only proposes data (a detail and a wrong first version of it, or an extra item that fits
a list); code places the error, builds the amendment from templates and checks the result.
"""

from __future__ import annotations

import random
import re

from utter_tools import common
from utter_tools.data.lists import ListPassage, parse, script
from utter_tools.data.script import Script
from utter_tools.llm import Chat

_PROPOSE_VALUE = common.prompt("propose-amend-value")

_PROPOSE_EXTRA = common.prompt("propose-amend-extra")

_VALUE_SCHEMA = {
    "type": "object",
    "properties": {"target": {"type": "string"}, "wrong": {"type": "string"}},
    "required": ["target", "wrong"],
}
_EXTRA_SCHEMA = {
    "type": "object",
    "properties": {"extra": {"type": "string"}},
    "required": ["extra"],
}

# How people take something back by reference. {wrong} and {right} are spoken as said.
VALUE = (
    "Actually, change {wrong} to {right}.",
    "Oh wait, I meant {right}, not {wrong}.",
    "Sorry, make that {right} instead of {wrong}.",
    "Let me fix that: it's {right}, not {wrong}.",
    "Actually, {wrong} should be {right}.",
)
DELETE = (
    "Actually, take {item} off the list.",
    "Oh, remove {item}.",
    "Scratch {item} from that.",
    "Never mind {item}.",
)
INSERT = ("Oh, and add {item} after {prev}.", "Wait, I forgot {item}, it goes after {prev}.")
POSITION = ("Actually, make the {nth} one {right}.", "Change the {nth} item to {right}.")
_NTH = [
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
]
_SENTENCE = re.compile(r"(?<=[.!?])\s+")


def _amend_prose(passage: str, rng: random.Random, llm: Chat, seed: int) -> tuple[str, dict] | None:
    p = llm.ask(_PROPOSE_VALUE, passage, _VALUE_SCHEMA, seed, max_tokens=200)
    if not p:
        return None
    target, wrong = p.get("target", "").strip(), p.get("wrong", "").strip()
    where = list(re.finditer(rf"(?<!\w){re.escape(target)}(?!\w)", passage))
    if (
        len(where) != 1
        or not wrong
        or wrong.lower() == target.lower()
        or wrong.lower() in passage.lower()
    ):
        return None
    m = where[0]
    said = passage[: m.start()] + wrong + passage[m.end() :]
    sentences = _SENTENCE.split(said)
    # The sentence holding the error, then the amendment one to all sentences later.
    at = len(_SENTENCE.split(said[: m.start()])) - 1
    later = rng.randint(at + 1, len(sentences)) if at + 1 <= len(sentences) else len(sentences)
    amendment = rng.choice(VALUE).format(wrong=wrong, right=target)
    text = " ".join(sentences[:later] + [amendment] + sentences[later:])
    return text, {"op": "value", "distance": str(later - at - 1)}


def _amend_list(
    lp: ListPassage, rng: random.Random, llm: Chat, seed: int
) -> tuple[str, dict] | None:
    variant = rng.choice(
        ("ordinals", "steps", "then") if lp.ordered else ("plain", "bullets", "also")
    )
    op = rng.choice(("value", "delete", "insert", "position"))
    items = list(lp.items)
    i = rng.randrange(len(items))
    if op == "delete":
        p = llm.ask(_PROPOSE_EXTRA, lp.markdown(), _EXTRA_SCHEMA, seed, max_tokens=200)
        extra = (p or {}).get("extra", "").strip()
        if not extra or extra.lower() in (it.lower() for it in items):
            return None
        said = ListPassage(lp.intro, items[:i] + [extra] + items[i:], lp.ordered, lp.outro)
        amendment = rng.choice(DELETE).format(item=extra)
    elif op == "insert":
        if i == 0:
            i = 1
        said = ListPassage(lp.intro, items[:i] + items[i + 1 :], lp.ordered, lp.outro)
        amendment = rng.choice(INSERT).format(
            item=items[i].rstrip("."), prev=items[i - 1].rstrip(".")
        )
    else:
        p = llm.ask(_PROPOSE_VALUE, items[i], _VALUE_SCHEMA, seed, max_tokens=200)
        wrong = (p or {}).get("wrong", "").strip()
        if not wrong or wrong.lower() == items[i].lower():
            return None
        said = ListPassage(lp.intro, items[:i] + [wrong] + items[i + 1 :], lp.ordered, lp.outro)
        right = items[i].rstrip(".")
        if op == "position" and i < len(_NTH):
            amendment = rng.choice(POSITION).format(nth=_NTH[i], right=right)
        else:
            op = "value"
            amendment = rng.choice(VALUE).format(wrong=wrong.rstrip("."), right=right)
    text = script(said, variant) + " " + amendment
    return text, {"op": op, "variant": variant, "distance": "end"}


def amend_script(passage: str, rng: random.Random, llm: Chat, seed: int) -> Script | None:
    """An amend rendition script for a passage (a list or prose), or None."""
    lp = parse(passage)
    out = _amend_list(lp, rng, llm, seed) if lp else _amend_prose(passage, rng, llm, seed)
    if out is None:
        return None
    text, tags = out
    return Script("amend", "natural", " ".join(text.split()), tags)
