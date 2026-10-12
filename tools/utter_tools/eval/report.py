"""One table comparing cleanup runs over the standard eval (plan.items): per set, ready to send where
graded, then no edit needed and characters to fix. Written as Markdown to
`results/report--<run>--<run>....md`, ready for docs/eval-results.md, and printed.
"""

from __future__ import annotations

import json

from utter_tools import config
from utter_tools.eval import judge
from utter_tools.eval.plan import Item


def _strict(item: Item, run: str) -> dict | None:
    path = config.results_dir() / "score" / f"{item.eval_set}--{item.asr_run}.json"
    if not path.is_file():
        return None
    return json.loads(path.read_text(encoding="utf-8")).get(run + item.suffix, {}).get("all")


def table(items: list[Item], runs: list[str], grader: str) -> str:
    head = "| Set | n | " + " | ".join(runs) + " |"
    rule = "|---|---|" + "---|" * len(runs)
    lines = [
        "Each cell: ready to send % (graded sets) · no edit needed % · characters to fix %.",
        "",
        head,
        rule,
    ]
    for item in items:
        cells, n = [], 0
        for run in runs:
            strict = _strict(item, run)
            if strict is None:
                cells.append("-")
                continue
            n = strict["n"]
            parts = [f"{strict['exact_pct']:.1f}", f"{strict['chars_to_fix_pct']:.1f}"]
            if item.graded:
                t = judge.tally(item.eval_set, run + item.suffix, grader)
                ready = f"**{t['ready_pct']:.1f}**" + (
                    f" ({t['ungraded']} ungraded)" if t["ungraded"] else ""
                )
                parts.insert(0, ready)
            cells.append(" · ".join(parts))
        lines.append(f"| {item.label()} | {n} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def write(items: list[Item], runs: list[str], grader: str) -> str:
    md = table(items, runs, grader)
    out = config.results_dir() / f"report--{'--'.join(runs)}.md"
    out.write_text(md, encoding="utf-8", newline="\n")
    print(md)
    print(f"written to {out}")
    return md
