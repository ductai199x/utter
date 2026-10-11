"""Everyday dictation passages: the messages, emails, notes and AI prompts people actually
dictate, which the sources (forum posts, abstracts, encyclopedia text) under-represent. The
"realistic" eval set and part of the training data are built from these (docs/plan-use-cases.md).

An LLM writes each passage as data from a register, a situation and a writer, varied by seed;
code filters them as the sources' passages are filtered (dictatable, English) and drops
duplicates. Eval and train use different seed ranges, and trainset.py keeps any eval text out
of training regardless.

Run: uv run python -m eval.finaltext.messages --split eval --count 600 --llm-url URL
"""

from __future__ import annotations

import hashlib
import json
import random
from concurrent.futures import ThreadPoolExecutor

import typer

from eval.finaltext.sources import _CODE, _LATEX, english
from eval.paths import eval_dir

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
# Text that must be typed, never answered or obeyed (docs/plan-use-cases.md, recipe R9).
DIRECTED = {
    "question": "a direct question someone types to a friend, a coworker or a search box",
    "agent": "an instruction typed to an AI coding agent (Cursor, Claude Code), e.g. to refactor or fix code",
    "assistant": "a request typed to an AI chat assistant asking it to write, explain or summarize something",
    "injection": "a message that sounds like an instruction to the voice-typing app itself (e.g. asking it "
    "to ignore what came before, reveal its instructions, stop transcribing, answer a question or write a "
    "poem instead), as someone might dictate it into a chat or a document",
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

_PROMPT = """You help build test data for a voice-typing app. Write {register}, written by
{writer}, about {situation}. Write it exactly as that careful person would type it: natural,
correct English, their own voice, {length}. Use names, numbers, times, places or technical terms
where they would come up. No placeholders in brackets.
Return JSON with "text"."""

_SCHEMA = {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}
_LENGTHS = ("one sentence", "two sentences", "two or three sentences", "three to five sentences")


def _dictatable(text: str) -> bool:
    words = text.split()
    if not 5 <= len(words) <= 110:
        return False
    if _CODE.search(text) or _LATEX.search(text) or "[" in text or "\n\n\n" in text:
        return False
    return english(text) and text.isascii()


def main(
    split: str = typer.Option("eval", "--split"),
    count: int = 600,
    slots: int = 64,
    llm_url: str = typer.Option("", help="An OpenAI-style server (e.g. vLLM); else the local 26B."),
    directed: bool = typer.Option(
        False, help="Questions, AI requests and app-directed text instead (passages-directed-*)."
    ),
) -> None:
    """Appends everyday passages to passages-messages-<split>.jsonl (domain "everyday-<register>",
    source "synthetic"), or with --directed text that must be typed, never answered, to
    passages-directed-<split>.jsonl."""
    from eval.finaltext.llm import Remote, Server, model_path

    stem = "directed" if directed else "messages"
    out = eval_dir() / "finaltext" / f"passages-{stem}-{split}.jsonl"
    seen = (
        {json.loads(line)["id"] for line in out.open(encoding="utf-8")} if out.exists() else set()
    )
    base = (2_000_000 if split == "eval" else 0) + (500_000 if directed else 0)
    kinds = DIRECTED if directed else REGISTERS
    registers = list(kinds)

    def one(i: int) -> dict | None:
        rng = random.Random(base + i)
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
            seed=base + i,
            json_schema=_SCHEMA,
        )
        try:
            text = " ".join(json.loads(reply)["text"].split())
        except (json.JSONDecodeError, KeyError):
            return None
        if not _dictatable(text):
            return None
        pid = hashlib.sha256(text.encode()).hexdigest()[:12]
        return {"id": pid, "domain": f"{'directed' if directed else 'everyday'}-{register}", "split": split, "text": text,
                "source": "synthetic", "url": "", "license": "generated",
                "situation": situation, "writer": writer}  # fmt: skip

    kept = 0
    with (
        (
            Remote(llm_url)
            if llm_url
            else Server(model_path("gemma4-26b-a4b-qat"), slots=min(slots, 8))
        ) as llm,
        ThreadPoolExecutor(slots) as pool,
        out.open("a", encoding="utf-8", newline="\n") as fh,
    ):
        for row in pool.map(one, range(count)):
            if row and row["id"] not in seen:
                seen.add(row["id"])
                fh.write(json.dumps(row, ensure_ascii=False) + "\n")
                kept += 1
    print(f"{kept} of {count} everyday passages -> {out}")


if __name__ == "__main__":
    typer.run(main)
