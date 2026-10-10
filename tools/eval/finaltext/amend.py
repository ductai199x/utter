"""Far-back edits for the "amend" rendition kind (docs/plan-use-cases.md, recipe R4): the
speaker says a detail wrong and fixes it later by reference, a sentence or more on, in the ways
people do: "actually, change the eggs to three boxes", "take the umbrella off the list", "oh,
and add bread after milk", "make the second one oat milk". The gold is the passage as meant.

The LLM only proposes data (a detail and a wrong first version of it, or an extra item that fits
a list); code places the error, builds the amendment from templates and checks the result.
"""

from __future__ import annotations

import json
import random
import re

from eval.finaltext.lists import ListPassage, parse, script

_PROPOSE_VALUE = """You help build test data for a dictation app. Given a passage, pick one
concrete detail a speaker could first say wrong and fix later: a quantity, a time, a day, a date,
a name, a place, an item. Return JSON with:
- "target": the detail, one to five words copied EXACTLY from the passage (it must occur once);
- "wrong": what the speaker first says instead: same kind of thing, plausible, different
  (for "three boxes of eggs": "one box of eggs"; for "Thursday": "Tuesday")."""

_PROPOSE_EXTRA = """You help build test data for a dictation app. Given a list, propose one extra
item the speaker might add by mistake and then remove: it fits the list, is not already in it,
and is written like the other items. Return JSON with "extra"."""

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


def _ask(llm, system: str, text: str, schema: dict, seed: int) -> dict | None:
    out = llm.chat(
        [{"role": "system", "content": system}, {"role": "user", "content": text}],
        max_tokens=200,
        temperature=0.9,
        seed=seed,
        json_schema=schema,
    )
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return None


def _amend_prose(passage: str, rng: random.Random, llm, seed: int):
    p = _ask(llm, _PROPOSE_VALUE, passage, _VALUE_SCHEMA, seed)
    if not p:
        return None
    target, wrong = p["target"].strip(), p["wrong"].strip()
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


def _amend_list(lp: ListPassage, rng: random.Random, llm, seed: int):
    variant = rng.choice(
        ("ordinals", "steps", "then") if lp.ordered else ("plain", "bullets", "also")
    )
    op = rng.choice(("value", "delete", "insert", "position"))
    items = list(lp.items)
    i = rng.randrange(len(items))
    if op == "delete":
        p = _ask(llm, _PROPOSE_EXTRA, lp.markdown(), _EXTRA_SCHEMA, seed)
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
        p = _ask(llm, _PROPOSE_VALUE, items[i], _VALUE_SCHEMA, seed)
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


def amend_script(passage: str, rng: random.Random, llm, seed: int):
    """An amend rendition script for a passage (a list or prose), or None."""
    from eval.finaltext.spoken import Script

    lp = parse(passage)
    out = _amend_list(lp, rng, llm, seed) if lp else _amend_prose(passage, rng, llm, seed)
    if out is None:
        return None
    text, tags = out
    return Script("amend", "natural", " ".join(text.split()), tags)
