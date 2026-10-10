"""Training pairs for the cleanup model, from a synthesized train set: what Parakeet heard (its
actual output on the TTS audio, errors and habits included) and the passage the speaker meant to
type.

Each row is in the exact chat format the product sends (crates/utter-llm/src/cleanup.rs,
messages_with): a system prompt, then a user message "Dictionary: ...\\n" (when there is one)
and "Dictated: <transcript>". The completion is the passage.

Run: uv run python -m eval.finaltext.trainset --set en-finaltext-train-kokoro --asr NAME
     --system PROMPT.md --out pairs.jsonl
"""

from __future__ import annotations

import json
import random
from dataclasses import dataclass
from pathlib import Path

import typer

from eval.finaltext.sources import english
from eval.paths import eval_dir


@dataclass
class Context:
    """Mirrors cleanup::Context in Rust: where the dictation goes."""

    app: str | None = None
    single_line: bool = False
    before: str | None = None
    after: str | None = None
    lists_off: bool = False


BEFORE_CHARS = 200
AFTER_CHARS = 50


def user_message(transcript: str, dictionary: list[str], ctx: Context | None = None) -> str:
    """Mirrors cleanup::messages_with in Rust, byte for byte."""
    ctx = ctx or Context()
    out = ""
    if ctx.app:
        out += f"App: {ctx.app}\n"
    if ctx.single_line:
        out += "Field: single-line\n"
    # JSON strings, as serde_json writes them (no ASCII escaping of other characters).
    if ctx.before:
        out += "Before: " + json.dumps(ctx.before[-BEFORE_CHARS:], ensure_ascii=False) + "\n"
    if ctx.after:
        out += "After: " + json.dumps(ctx.after[:AFTER_CHARS], ensure_ascii=False) + "\n"
    if ctx.lists_off:
        out += "Lists: off\n"
    if dictionary:
        out += "Dictionary: " + ", ".join(dictionary) + "\n"
    return out + "Dictated: " + transcript.strip()


def _passage_of(clip_id: str) -> str:
    """A clip id is "<passage id>-<kind>"."""
    return clip_id.rsplit("-", 1)[0]


def main(
    eval_set: str = typer.Option(..., "--set"),
    asr_name: str = typer.Option(..., "--asr"),
    system: Path = typer.Option(...),
    out: Path = typer.Option(...),
    holdout: float = typer.Option(
        0.02, help="Fraction held out for the training run's own eval loss."
    ),
    seed: int = 0,
) -> None:
    prompt = system.read_text(encoding="utf-8")
    manifest = {
        r["id"]: r
        for r in (
            json.loads(line)
            for line in (eval_dir() / eval_set / "manifest.jsonl").open(encoding="utf-8")
        )
    }
    hyp_path = eval_dir() / "results" / "asr" / asr_name / f"{eval_set}.jsonl"
    hyps = [json.loads(line) for line in hyp_path.read_text(encoding="utf-8").splitlines() if line]
    rows = []
    for h in hyps:
        m = manifest.get(h["id"])
        # Clips synthesized before sources.py filtered other languages are skipped here.
        if m is None or not h["hyp"].strip() or not english(m["gold"]):
            continue
        rows.append(
            {
                "id": h["id"],
                "kind": m["kind"],
                "prompt": [
                    {"role": "system", "content": prompt},
                    {"role": "user", "content": user_message(h["hyp"], [])},
                ],
                "completion": [{"role": "assistant", "content": m["gold"]}],
            }
        )
    random.Random(seed).shuffle(rows)
    # Whole passages are held out: a passage's other renditions in training would make the
    # eval loss look better than it is.
    passages = sorted({_passage_of(r["id"]) for r in rows})
    held = set(random.Random(seed).sample(passages, int(len(passages) * holdout)))
    hold = [r for r in rows if _passage_of(r["id"]) in held]
    train = [r for r in rows if _passage_of(r["id"]) not in held]
    out.parent.mkdir(parents=True, exist_ok=True)
    for path, part in ((out.with_suffix(".eval.jsonl"), hold), (out, train)):
        with path.open("w", encoding="utf-8") as fh:
            for r in part:
                fh.write(json.dumps(r, ensure_ascii=False) + "\n")
        print(f"{len(part)} rows -> {path}")


if __name__ == "__main__":
    typer.run(main)
