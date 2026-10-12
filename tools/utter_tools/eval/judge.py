"""Ready to send: the share of dictations a careful writer would send exactly as typed.

"No edit needed" (exact match with the gold) is strict: an optional comma or "OK" for "okay"
counts as a miss, though nobody would edit it. This grades each miss with an LLM against a
rubric: acceptable (only equally valid choices differ from the intended text) or needs an edit,
and the cause. Causes separate what training can fix (the model's mistakes) from what it cannot
(a word the recognizer misheard beyond recovery). Exact matches are acceptable without asking.
The rubric was calibrated on Tai's review of 20 verdicts (docs/eval-results.md, 2026-10-11).

Verdicts go to `results/judge/<set>--<run>--<grader>.jsonl`, one per graded dictation; a rerun
grades only what is new. Graders grade a little differently, so results are kept per grader and
only runs graded the same way are compared.
"""

from __future__ import annotations

import json
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from utter_tools import config, llm
from utter_tools.eval.sets import llm_path, manifest, read_jsonl

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


def same(a: str, b: str) -> bool:
    return " ".join(a.split()) == " ".join(b.split())


def verdicts_path(eval_set: str, run: str, grader: str) -> Path:
    return config.results_dir() / "judge" / f"{eval_set}--{run}--{grader}.jsonl"


def _cases(eval_set: str, run: str) -> list[dict]:
    rows = {r["id"]: r for r in manifest(eval_set)}
    return [
        {
            "id": o["id"],
            "heard": o["input"],
            "gold": rows[o["id"]]["final_text"],
            "typed": o["output"],
        }
        for o in read_jsonl(llm_path(run, eval_set))
    ]


def grade(eval_set: str, run: str, chat: llm.Chat, grader: str, workers: int = 8) -> None:
    """Grades every non-exact output of `run` on `eval_set` not graded yet."""
    out = verdicts_path(eval_set, run, grader)
    done = {r["id"] for r in read_jsonl(out)} if out.exists() else set()
    todo = [
        c for c in _cases(eval_set, run) if c["id"] not in done and not same(c["typed"], c["gold"])
    ]
    print(f"{eval_set} / {run}: {len(todo)} to grade")
    if not todo:
        return

    def one(c: dict) -> dict:
        user = f"Heard: {c['heard']}\n\nMeant: {c['gold']}\n\nTyped: {c['typed']}"
        reply = chat.chat(
            [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}],
            max_tokens=600,
            temperature=0.0,
            json_schema=SCHEMA,
        )
        try:
            return {"id": c["id"], **json.loads(reply)}
        except json.JSONDecodeError:
            return {"id": c["id"], "verdict": "needs_edit", "cause": "other", "note": "unreadable"}

    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("a", encoding="utf-8", newline="\n") as fh, ThreadPoolExecutor(workers) as pool:
        for r in pool.map(one, todo):
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
            fh.flush()


def tally(eval_set: str, run: str, grader: str) -> dict:
    """Ready to send overall and by category, and the causes of what needs an edit."""
    categories = {r["id"]: r["category"] for r in manifest(eval_set)}
    path = verdicts_path(eval_set, run, grader)
    graded = {r["id"]: r for r in read_jsonl(path)} if path.exists() else {}
    by_category: dict[str, Counter] = {}
    causes: Counter = Counter()
    ungraded = 0
    for c in _cases(eval_set, run):
        t = by_category.setdefault(categories[c["id"]], Counter())
        t["n"] += 1
        if same(c["typed"], c["gold"]):
            t["exact"] += 1
            t["ok"] += 1
        elif (g := graded.get(c["id"])) is None:
            ungraded += 1
        elif g["verdict"] == "acceptable":
            t["ok"] += 1
        else:
            causes[g["cause"]] += 1
    total: Counter = Counter()
    for t in by_category.values():
        total.update(t)

    def pct(t: Counter, key: str) -> float:
        return round(100 * t[key] / max(1, t["n"]), 2)

    return {
        "ready_pct": pct(total, "ok"),
        "exact_pct": pct(total, "exact"),
        "n": total["n"],
        "ungraded": ungraded,
        "by_category": {
            k: {"ready_pct": pct(t, "ok"), "n": t["n"]} for k, t in sorted(by_category.items())
        },
        "causes": dict(causes.most_common()),
    }
