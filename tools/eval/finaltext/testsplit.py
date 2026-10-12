"""dictation-to-text's test split as an eval set, so a cleanup model is graded on it like on the
speech sets: what the recognizer heard (`dictated`) in, the row's `final_text` as the gold.

Writes, under the eval dir:
- `dictation-to-text-test/manifest.jsonl`: one row per test row (`id` prefixed `t:`, so no passage
  file's gold can stand in for it; `gold` and `set_gold` the row's final text);
- `results/asr/testsplit/dictation-to-text-test.jsonl`: the heard text, as a recognizer run;
- `results/llm/<name>/dictation-to-text-test-input.jsonl` per `--cleanup` name: the cases for
  `utter-cli llm-bench`, each with the row's dictionary and lists setting, as the app sends them.

Then: `utter-cli llm-bench --gguf M --name NAME --cases <that file> --prompt cleanup-ft.md ...`,
`report score --set dictation-to-text-test --asr testsplit --gold all --cleanup NAME`, and
`judge --set dictation-to-text-test --cleanup NAME`.

Run: uv run python -m eval.finaltext.testsplit PARQUET --cleanup NAME [--cleanup NAME ...]
"""

from __future__ import annotations

import json
from pathlib import Path

import pyarrow.parquet as pq
import typer

from eval.paths import eval_dir

SET = "dictation-to-text-test"


def _write(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as fh:
        for r in rows:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")


def main(parquet: Path, cleanup: list[str] = typer.Option([], "--cleanup")) -> None:
    cols = ["id", "category", "domain", "dictated", "final_text", "dictionary", "lists"]
    rows = pq.read_table(parquet, columns=cols).to_pylist()
    manifest = [
        {
            "id": f"t:{r['id']}",
            "gold": r["final_text"],
            "set_gold": r["final_text"],
            "domain": r["domain"],
            "kind": r["category"],
            "tags": {},
        }
        for r in rows
    ]
    _write(eval_dir() / SET / "manifest.jsonl", manifest)
    heard = [{"id": f"t:{r['id']}", "hyp": r["dictated"]} for r in rows]
    _write(eval_dir() / "results" / "asr" / "testsplit" / f"{SET}.jsonl", heard)
    cases = []
    for r in rows:
        case = {"id": f"t:{r['id']}", "input": r["dictated"]}
        if r["dictionary"]:
            case["dictionary"] = r["dictionary"]
        if not r["lists"]:
            case["lists_off"] = True
        cases.append(case)
    for name in cleanup:
        _write(eval_dir() / "results" / "llm" / name / f"{SET}-input.jsonl", cases)
    dictionary = sum(1 for c in cases if "dictionary" in c)
    lists_off = sum(1 for c in cases if c.get("lists_off"))
    print(f"{len(rows)} test rows ({dictionary} with a dictionary, {lists_off} with lists off)")


if __name__ == "__main__":
    typer.run(main)
