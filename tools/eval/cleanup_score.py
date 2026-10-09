"""Score cleanup outputs written by `utter-cli llm-bench` (rules: docs/cleanup-spec.md).

Two kinds of case file:
- `cleanup_cases.jsonl` (curated, committed): each case has an expected output and checks.
  Scored by exact match, by its checks (`must_contain`: case-sensitive substrings;
  `must_not_contain`: case-insensitive whole words or phrases), by word error rate against the
  expected text (lower-cased, punctuation removed: how far a miss is), and by added words.
- `cleanup_real.jsonl` (built by `build-real` from the speech model's own transcripts of
  spontaneous speech; no expected output): scored by rules code can check. Fillers left,
  words added that the speaker never said, the share of the speaker's other words kept, and
  outputs that look like answers rather than cleanups.

"Added words" are output words found in neither the input nor the expected output, after
lower-casing and removing punctuation; numbers are exempt (cleanup writes "twenty" as "20").

Run:
  uv run python -m eval.cleanup_score build-real
  uv run python -m eval.cleanup_score report [--models a,b]
"""

from __future__ import annotations

import json
import re
import statistics
from pathlib import Path

import jiwer
import typer
from rich.console import Console
from rich.table import Table

from eval.paths import eval_dir

FILLERS = {"um", "uh", "er", "ah", "hmm", "erm", "uhm", "mm"}
WORD = re.compile(r"[\w'@.\-/:$%&]+", re.UNICODE)
# Openings that mean the model answered or commented instead of cleaning up.
ASSISTANT_OPENINGS = (
    "sure",
    "certainly",
    "here is",
    "here's the",
    "of course",
    "i can't",
    "i cannot",
    "as an ai",
)

console = Console()
app = typer.Typer(add_completion=False)


def results_dir() -> Path:
    return eval_dir() / "results" / "llm"


def words(text: str) -> list[str]:
    """Lower-cased words with surrounding punctuation stripped."""
    out = []
    for w in WORD.findall(text.lower()):
        w = w.strip(".,:;!?\"'()-/")
        if w:
            out.append(w)
    return out


def is_number(w: str) -> bool:
    return any(ch.isdigit() for ch in w)


def contains_phrase(text: str, phrase: str) -> bool:
    """Case-insensitive match of a word or phrase on word boundaries (where it has word
    characters at its ends)."""
    p = re.escape(phrase.lower())
    left = r"(?<!\w)" if phrase[:1].isalnum() else ""
    right = r"(?!\w)" if phrase[-1:].isalnum() else ""
    return re.search(left + p + right, text.lower()) is not None


def added_words(output: str, *sources: str) -> list[str]:
    allowed = set()
    for s in sources:
        allowed.update(words(s))
    return [w for w in words(output) if w not in allowed and not is_number(w)]


def score_curated(rows: list[dict]) -> dict:
    per_cat: dict[str, list[bool]] = {}
    exact, checks, wers, added_cases, failures = 0, 0, [], 0, []
    for r in rows:
        out, exp = r["output"], r["expected"]
        is_exact = " ".join(out.split()) == " ".join(exp.split()) and out.count("\n") == exp.count(
            "\n"
        )
        ok = all(s in out for s in r.get("must_contain", [])) and not any(
            contains_phrase(out, s) for s in r.get("must_not_contain", [])
        )
        exp_n, out_n = " ".join(words(exp)), " ".join(words(out))
        wers.append(jiwer.wer(exp_n, out_n) if exp_n else float(bool(out_n)))
        extra = added_words(out, r["input"], exp)
        exact += is_exact
        checks += ok
        added_cases += bool(extra)
        per_cat.setdefault(r["category"], []).append(ok)
        if not ok:
            failures.append((r["id"], out))
    n = len(rows)
    return {
        "n": n,
        "exact": exact / n,
        "checks": checks / n,
        "wer": statistics.mean(wers),
        "added": added_cases / n,
        "per_cat": {k: sum(v) / len(v) for k, v in per_cat.items()},
        "failures": failures,
    }


def score_real(rows: list[dict]) -> dict:
    filler_left, added, kept, answered = 0, 0, [], 0
    for r in rows:
        inp, out = r["input"], r["output"]
        out_w = words(out)
        filler_left += any(w in FILLERS for w in out_w)
        added += bool(added_words(out, inp))
        content = [w for w in words(inp) if w not in FILLERS]
        if content:
            out_set = set(out_w)
            kept.append(sum(w in out_set or is_number(w) for w in content) / len(content))
        too_long = len(out_w) > 1.5 * len(words(inp)) + 3
        answered += too_long or out.strip().lower().startswith(ASSISTANT_OPENINGS)
    n = len(rows)
    return {
        "n": n,
        "filler_left": filler_left / n,
        "added": added / n,
        "kept": statistics.mean(kept) if kept else float("nan"),
        "answered": answered / n,
    }


def latency(rows: list[dict]) -> tuple[float, float]:
    walls = sorted(r["wall_ms"] for r in rows)
    return statistics.median(walls), walls[min(len(walls) - 1, int(0.95 * len(walls)))]


@app.command("build-real")
def build_real(min_words: int = 5) -> None:
    """Cases from the speech model's transcripts of spontaneous English speech (AMI meetings,
    Monsoon conversations): the inputs cleanup actually gets."""
    src = eval_dir() / "results" / "asr" / "parakeet-ultra-rust"
    out_dir = eval_dir() / "cleanup"
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    for s in ("en-ami", "en-monsoon"):
        for line in (src / f"{s}.jsonl").open(encoding="utf-8"):
            r = json.loads(line)
            if len(r["hyp"].split()) >= min_words:
                rows.append({"id": f"{s}/{r['id']}", "category": s, "input": r["hyp"]})
    path = out_dir / "cleanup_real.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    console.print(f"{len(rows)} cases -> {path}")


@app.command()
def report(
    models: str = typer.Option("", help="Comma-separated result names (default: all)."),
    failures: str = typer.Option("", help="Print the failed curated cases of this model."),
) -> None:
    names = (
        models.split(",")
        if models
        else sorted(p.name for p in results_dir().iterdir() if p.is_dir())
    )
    t = Table(title="Cleanup: curated cases (checks %, higher is better) and real speech")
    for col in (
        "model",
        "exact",
        "checks",
        "WER vs exp",
        "added",
        "fillers",
        "backtrack",
        "lists",
        "spoken",
        "numbers",
        "clean",
        "no-answer",
        "dict",
        "p50 ms",
        "p95 ms",
        "real: fillers left",
        "real: added",
        "real: kept",
        "real: answered",
        "real p50",
    ):
        t.add_column(col, justify="left" if col == "model" else "right")
    for name in names:
        d = results_dir() / name
        cur_path, real_path = d / "cleanup_cases.jsonl", d / "cleanup_real.jsonl"
        cells: list[str] = [name]
        if cur_path.exists():
            rows = [json.loads(line) for line in cur_path.open(encoding="utf-8")]
            s = score_curated(rows)
            p50, p95 = latency(rows)
            pc = s["per_cat"]
            cells += [
                f"{s['exact'] * 100:.0f}",
                f"{s['checks'] * 100:.0f}",
                f"{s['wer'] * 100:.1f}",
                f"{s['added'] * 100:.0f}",
            ]
            cells += [
                f"{pc.get(c, float('nan')) * 100:.0f}"
                for c in (
                    "fillers",
                    "backtrack",
                    "lists",
                    "spoken-formatting",
                    "numbers",
                    "clean",
                    "do-not-answer",
                    "dictionary",
                )
            ]
            cells += [f"{p50:.0f}", f"{p95:.0f}"]
            if failures == name:
                for fid, out in s["failures"]:
                    console.print(f"[yellow]{fid}[/yellow]: {out!r}")
        else:
            cells += [""] * 14
        if real_path.exists():
            rows = [json.loads(line) for line in real_path.open(encoding="utf-8")]
            s = score_real(rows)
            cells += [
                f"{s['filler_left'] * 100:.1f}",
                f"{s['added'] * 100:.1f}",
                f"{s['kept'] * 100:.1f}",
                f"{s['answered'] * 100:.1f}",
                f"{latency(rows)[0]:.0f}",
            ]
        else:
            cells += [""] * 5
        t.add_row(*cells)
    console.print(t)


if __name__ == "__main__":
    app()
