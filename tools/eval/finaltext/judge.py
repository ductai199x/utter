"""How many dictations a user would accept as typed, and why the rest need an edit.

"No edit needed" (exact match with the gold) is strict: an optional comma or "OK" for "okay"
counts as a miss, though nobody would edit it. This grades each miss with an LLM against a
rubric: acceptable (only equally valid choices differ from the intended text) or needs an edit,
and the cause. Causes separate what training can fix (the model's mistakes) from what it cannot
(a word the recognizer misheard beyond recovery). Exact matches are acceptable without asking.

The grader's verdicts are checked against Tai's on a sample before the numbers are trusted.

Run: uv run python -m eval.finaltext.judge --set en-finaltext-kokoro --cleanup ft4 [--llm-url URL]
"""

from __future__ import annotations

import json
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import typer

from eval.paths import eval_dir

CAUSES = (
    "style",  # only equally valid choices differ (never a reason to edit)
    "recognition",  # a word the recognizer misheard that the text gives no way to recover
    "punctuation",
    "capitalization",
    "grammar",
    "correction",  # a self-correction, retraction or edit applied wrongly or not at all
    "list",  # list formatting
    "numbers-symbols",
    "words",  # words dropped, added or changed otherwise
    "other",
)

SYSTEM = """You grade a voice-typing app. You get what the speech recognizer heard, the text
the speaker meant to type, and what the app typed. Decide whether a careful writer would send the
app's text exactly as it is.

Compare Typed with Meant, word by word. "Heard" is only context, to tell whether a wrong word
was misheard; never judge the Heard text. First list every difference between Typed and Meant
("differences"), each as "typed X / meant Y". Judge only those differences.

- "acceptable": every difference from the intended text is an equally valid choice: an optional
  comma, a semicolon or dash instead of a period between sentences, "OK" for "okay", straight or
  curly quotes, a number written in another standard way, a list marker style. Nothing a reader
  would call wrong. Also acceptable: a difference where Meant itself has a clear typo or grammar
  error (real people's text sometimes does) and Typed is the correct version of exactly that
  part, with the same meaning. Punctuation is a writer's choice unless it garbles the sentence
  or changes its meaning: an aside in commas or none instead of parentheses, a period instead of
  an ellipsis or an exclamation mark, a period on a question the wording does not mark as one
  ("Or you are afraid of bugs."), all acceptable.
- "needs_edit": at least one difference is wrong: a missing, extra or changed word, a misspelled
  name, a grammar error, punctuation that changes or garbles the reading, a correction applied
  wrongly or left in, a list that should be a list or should not be.

If it needs an edit, name the main cause:
style (only if acceptable), recognition (the recognizer misheard a word and nothing in what it
heard lets anyone recover it), punctuation, capitalization, grammar, correction, list,
numbers-symbols, words, other.

Return JSON: {"differences": [...], "verdict": "acceptable" | "needs_edit", "cause": ...,
"note": a few words}."""

SCHEMA = {
    "type": "object",
    "properties": {
        "differences": {"type": "array", "items": {"type": "string"}},
        "verdict": {"enum": ["acceptable", "needs_edit"]},
        "cause": {"enum": list(CAUSES)},
        "note": {"type": "string"},
    },
    "required": ["differences", "verdict", "cause", "note"],
}


def _rows(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def _golds() -> dict[str, str]:
    root = eval_dir() / "finaltext"
    golds = {}
    for stem in (
        "passages-eval",
        "passages-lists-eval",
        "passages-messages-eval",
        "passages-directed-eval",
    ):
        path = root / f"{stem}.jsonl"
        if path.exists():
            golds |= {p["id"]: p["text"] for p in _rows(path)}
    return golds


def _same(a: str, b: str) -> bool:
    return " ".join(a.split()) == " ".join(b.split())


def main(
    eval_set: str = typer.Option(..., "--set"),
    cleanup: str = typer.Option(..., "--cleanup", help="A cleanup run (results/llm/<name>)."),
    model: str = "gemma4-26b-a4b-qat",
    slots: int = 8,
    llm_url: str = typer.Option("", help="An OpenAI-style server instead of a local llama-server."),
    grader: str = typer.Option(
        "", help="A name for the grader when it is not the local 26B GGUF (kept apart in results)."
    ),
) -> None:
    from eval.finaltext.llm import Remote, Server, model_path

    manifest = {r["id"]: r for r in _rows(eval_dir() / eval_set / "manifest.jsonl")}
    golds = _golds()
    outs = _rows(eval_dir() / "results" / "llm" / cleanup / f"{eval_set}-input.jsonl")
    cases = []
    for o in outs:
        # A set without passage files (dictation-to-text's test split, testsplit.py) carries
        # each dictation's gold in its manifest.
        gold = golds.get(o["id"].rsplit("-", 1)[0]) or manifest.get(o["id"], {}).get("set_gold")
        if gold is not None and o["id"] in manifest:
            cases.append({"id": o["id"], "heard": o["input"], "gold": gold, "typed": o["output"]})

    suffix = f"--{grader}" if grader else ""
    out_path = eval_dir() / "results" / "finaltext" / f"judge-{eval_set}--{cleanup}{suffix}.jsonl"
    done = {r["id"]: r for r in _rows(out_path)} if out_path.exists() else {}
    todo = [c for c in cases if c["id"] not in done and not _same(c["typed"], c["gold"])]
    print(
        f"{len(cases)} dictations: {len(cases) - len(todo) - len(done)} exact, {len(todo)} to grade"
    )

    server = Remote(llm_url) if llm_url else Server(model_path(model), slots=slots)
    with server as llm, out_path.open("a", encoding="utf-8", newline="\n") as fh:

        def grade(c: dict) -> dict:
            user = f"Heard: {c['heard']}\n\nMeant: {c['gold']}\n\nTyped: {c['typed']}"
            reply = llm.chat(
                [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}],
                max_tokens=600,
                temperature=0.0,
                json_schema=SCHEMA,
            )
            try:
                return {"id": c["id"], **json.loads(reply)}
            except json.JSONDecodeError:
                return {
                    "id": c["id"],
                    "verdict": "needs_edit",
                    "cause": "other",
                    "note": "unreadable",
                }

        with ThreadPoolExecutor(slots) as pool:
            for r in pool.map(grade, todo):
                fh.write(json.dumps(r, ensure_ascii=False) + "\n")
                fh.flush()
                done[r["id"]] = r

    report(cases, done, manifest, eval_set, cleanup)


def report(cases: list[dict], graded: dict, manifest: dict, eval_set: str, cleanup: str) -> None:
    by_kind: dict[str, Counter] = {}
    causes: Counter = Counter()
    for c in cases:
        kind = manifest[c["id"]]["kind"]
        tally = by_kind.setdefault(kind, Counter())
        tally["n"] += 1
        if _same(c["typed"], c["gold"]):
            tally["exact"] += 1
            tally["ok"] += 1
            continue
        g = graded.get(c["id"])
        if g and g["verdict"] == "acceptable":
            tally["ok"] += 1
        elif g:
            causes[g["cause"]] += 1
    total = Counter()
    for t in by_kind.values():
        total.update(t)
    print(f"{eval_set} / {cleanup}: acceptable as typed")
    for kind, t in sorted(by_kind.items()) + [("all", total)]:
        print(
            f"  {kind:12} {100 * t['ok'] / t['n']:5.1f}%  (exact {100 * t['exact'] / t['n']:4.1f}%, n={t['n']})"
        )
    need = sum(causes.values())
    print(f"  needs an edit: {need} ({100 * need / max(1, total['n']):.1f}%), by cause:")
    for cause, n in causes.most_common():
        print(f"    {cause:16} {n:5}  {100 * n / max(1, total['n']):5.1f}% of all")


if __name__ == "__main__":
    typer.run(main)
