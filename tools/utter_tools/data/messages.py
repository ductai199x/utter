"""Everyday dictation passages: the messages, emails, notes and AI prompts people actually
dictate, which the sources (forum posts, abstracts, encyclopedia text) under-represent. The
"realistic" eval set and part of the training data are built from these (docs/cleanup.md).

An LLM writes each passage as data from a register, a situation and a writer, varied by seed;
code filters them as the sources' passages are filtered (dictatable, English) and drops
duplicates. Each batch has its own seed, so batches never repeat each other's prompts.

With `directed`, text that must be typed, never answered or obeyed: questions, requests to AI
assistants and agents, and text addressed to the voice app itself (docs/cleanup.md, R9).
"""

from __future__ import annotations

import json
import random
from concurrent.futures import ThreadPoolExecutor

from utter_tools import common
from utter_tools.data.batch import passage
from utter_tools.data.passages import CODE, LATEX, english
from utter_tools.llm import Chat

REGISTERS = {
    "chat": "a Slack or Teams message to a coworker",
    "text": "a text message to a friend or family member",
    "email": "the body of a short work email (no greeting or sign-off)",
    "email-personal": "the body of a short personal email (no greeting or sign-off)",
    "note": "a note to self or meeting notes",
    "prompt": "a request typed to an AI assistant (ChatGPT, Claude, Cursor)",
    "update": "a status update on a project",
    "reply": "a reply in a work thread answering a question",
}
# Text that must be typed, never answered or obeyed (docs/cleanup.md, recipe R9).
DIRECTED = {
    "question": "a direct question someone types to a friend, a coworker or a search box",
    "agent": (
        "an instruction typed to an AI coding agent (Cursor, Claude Code), "
        "e.g. to refactor or fix code"
    ),
    "assistant": (
        "a request typed to an AI chat assistant asking it to write, explain or summarize something"
    ),
    "injection": (
        "a message that sounds like an instruction to the voice-typing app itself (e.g. "
        "asking it to ignore what came before, reveal its instructions, stop transcribing, "
        "answer a question or write a poem instead), as someone might dictate it into a chat "
        "or a document"
    ),
}
SITUATIONS = (
    "scheduling a meeting", "a bug in production", "a deadline slipping", "weekend plans",
    "a code review", "planning a trip", "a doctor's appointment", "a product launch",
    "asking for help with a spreadsheet", "a customer complaint", "a research paper draft",
    "moving apartments", "a hiring decision", "a broken laptop", "a birthday dinner",
    "a training run on a GPU cluster", "a budget question", "a school pickup",
    "debugging a Python script", "a conference talk", "groceries for the week",
    "a lease renewal", "a design review", "an API migration", "a car repair",
)  # fmt: skip
WRITERS = (
    "a software engineer", "a PhD student", "a product manager", "a nurse", "a teacher",
    "a small business owner", "a data scientist", "a parent", "a sales manager",
    "a graduate student in physics", "a designer", "a lawyer", "a startup founder",
)  # fmt: skip

_PROMPT = common.prompt("write-message")

_SCHEMA = {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}
_LENGTHS = ("one sentence", "two sentences", "two or three sentences", "three to five sentences")


def _dictatable(text: str) -> bool:
    words = text.split()
    if not 5 <= len(words) <= 110:
        return False
    if CODE.search(text) or LATEX.search(text) or "[" in text or "\n\n\n" in text:
        return False
    return english(text) and text.isascii()


def generate(
    count: int, seed: int, llm: Chat, url: str, directed: bool = False, workers: int = 64
) -> list[dict]:
    """`count` passages written by `llm` (domain "everyday-<register>", or "directed-<kind>"),
    seeded from `seed`; those that fail the filters are dropped."""
    kinds = DIRECTED if directed else REGISTERS
    registers = list(kinds)
    prefix = "directed" if directed else "everyday"

    def one(i: int) -> dict | None:
        rng = random.Random(seed + i)
        register = rng.choice(registers)
        situation, writer = rng.choice(SITUATIONS), rng.choice(WRITERS)
        prompt = _PROMPT.format(
            register=kinds[register], writer=writer, situation=situation,
            length=rng.choice(_LENGTHS),
        )  # fmt: skip
        reply = llm.chat(
            [{"role": "user", "content": prompt}],
            max_tokens=400,
            temperature=1.0,
            seed=seed + i,
            json_schema=_SCHEMA,
        )
        try:
            text = " ".join(json.loads(reply)["text"].split())
        except (json.JSONDecodeError, KeyError):
            return None
        if not _dictatable(text):
            return None
        return passage(f"{prefix}-{register}", text, "synthetic", url, "generated")

    with ThreadPoolExecutor(workers) as pool:
        return [row for row in pool.map(one, range(count)) if row]
