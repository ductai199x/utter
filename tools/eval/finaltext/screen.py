"""Screens passages for gold quality: is this how a careful writer would type it?

The eval scores Utter against the passage, so the passage must be well-formed: a typo, a
lowercase "ubuntu", an ALL-CAPS emphasis, or chat shorthand ("d/l", "u", "ur") in the gold would
mark a correct output as wrong. Regex filters (sources.py) catch the mechanical cases; this
catches the rest with the local LLM, conservatively (clean unless a clear problem).

Run: uv run python -m eval.finaltext.screen [--split eval]
"""

from __future__ import annotations

import json
from concurrent.futures import ThreadPoolExecutor

import typer

from eval.paths import eval_dir

SYSTEM = """You check sentences that will be the answer key for a voice-typing app: the app
hears someone dictate the sentence and must type it exactly. So the sentence must be written the
way a careful, fluent writer types it. Flag only clear problems:

- typo: a misspelled word ("hasle", "aplication", "recieve");
- capitalization: a proper noun, product or name in lowercase ("ubuntu", "google"), a sentence
  starting in lowercase, or a word in capitals for emphasis ("you WANT this"); acronyms like
  "GPU" or "LTS" are fine;
- shorthand: chat abbreviations a speaker would say as full words ("d/l" for download, "u" for
  you, "ur", "pls", "thx", "b/c"); common written abbreviations ("e.g.", "etc.", "vs.") are fine;
- punctuation: clearly missing or wrong punctuation (no punctuation between two sentences,
  "its" for "it's", a stray symbol);
- not_prose: content nobody would dictate (a citation, a table row, a file listing, code).

Style choices are not problems: informal tone, contractions, sentence fragments that read
naturally, British spelling, an Oxford comma or not. When unsure, it is clean.

Return JSON: {"clean": true or false, "problems": [{"kind": ..., "text": the exact words}]}."""

SCHEMA = {
    "type": "object",
    "properties": {
        "clean": {"type": "boolean"},
        "problems": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "kind": {
                        "enum": ["typo", "capitalization", "shorthand", "punctuation", "not_prose"]
                    },
                    "text": {"type": "string"},
                },
                "required": ["kind", "text"],
            },
        },
    },
    "required": ["clean", "problems"],
}


def main(split: str = "eval", model: str = "gemma4-26b-a4b-qat", slots: int = 6) -> None:
    from eval.finaltext.llm import Server, model_path

    root = eval_dir() / "finaltext"
    passages = [
        json.loads(line) for line in (root / f"passages-{split}.jsonl").open(encoding="utf-8")
    ]
    out = root / f"screen-{split}.jsonl"
    done = (
        {json.loads(line)["id"] for line in out.open(encoding="utf-8")} if out.exists() else set()
    )
    todo = [p for p in passages if p["id"] not in done]

    with Server(model_path(model), slots=slots) as llm, out.open("a", encoding="utf-8") as fh:

        def one(p: dict) -> dict:
            reply = llm.chat(
                [{"role": "system", "content": SYSTEM}, {"role": "user", "content": p["text"]}],
                max_tokens=300,
                temperature=0.0,
                json_schema=SCHEMA,
            )
            r = json.loads(reply)
            # A verdict of "not clean" with no problem named is not a verdict.
            clean = bool(r["clean"]) or not r["problems"]
            return {"id": p["id"], "clean": clean, "problems": r["problems"]}

        flagged = 0
        with ThreadPoolExecutor(slots) as pool:
            for i, r in enumerate(pool.map(one, todo), 1):
                fh.write(json.dumps(r, ensure_ascii=False) + "\n")
                fh.flush()
                flagged += not r["clean"]
                if i % 100 == 0:
                    print(f"{i}/{len(todo)} screened, {flagged} flagged", flush=True)
    print(f"done -> {out}")


if __name__ == "__main__":
    typer.run(main)
