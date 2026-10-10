"""Runs the final-text eval and reports it, by domain, kind and tag.

Stages (each can be rerun alone; outputs land under <eval dir>/results/):

  asr      Parakeet on the synthesized set, through Utter's Rust runtime (`utter-cli bench`).
  cleanup  The product's cleanup on Parakeet's output, through the Rust client with the
           product's server settings (`utter-cli llm-bench`).
  score    Every system (Parakeet alone, and each cleanup run) against the gold passages.

Run: uv run python -m eval.finaltext.report score --set en-finaltext-kokoro --asr NAME
     [--cleanup NAME ...]
"""

from __future__ import annotations

import json
import subprocess
from collections import defaultdict
from pathlib import Path

import typer
from rich.console import Console
from rich.table import Table

from eval.finaltext.score import Score, prf, score
from eval.finaltext.spans import spans
from eval.paths import eval_dir

app = typer.Typer(add_completion=False)
console = Console(width=160)

REPO = Path(__file__).resolve().parents[3]
CLI = REPO / "target" / "release" / "utter-cli.exe"
# The product's cleanup settings (crates/utter-llm/src/cleanup.rs, SERVER_ARGS).
PRODUCT_SERVER_ARGS = [
    "--swa-full",
    "--spec-type",
    "ngram-simple",
    "--spec-ngram-simple-size-n",
    "3",
    "--spec-ngram-simple-size-m",
    "6",
]
SCREEN_EXCLUDES = {"typo", "capitalization", "shorthand", "not_prose"}
MARKS_REPORTED = [".", ",", "?", "!", ":", ";", "—", "(", ")", '"', "'", "\n", "\n\n"]
MARK_NAMES = {"—": "dash", "\n": "line break", "\n\n": "paragraph", '"': "quote", "'": "apostrophe"}


def _results(kind: str, name: str) -> Path:
    return eval_dir() / "results" / kind / name


def _manifest(eval_set: str) -> list[dict]:
    path = eval_dir() / eval_set / "manifest.jsonl"
    return [json.loads(line) for line in path.open(encoding="utf-8")]


@app.command()
def asr(eval_set: str = typer.Option(..., "--set"), name: str = "parakeet-ultra-rust") -> None:
    subprocess.run([str(CLI), "bench", "--sets", eval_set, "--name", name], check=True)


@app.command()
def cleanup(
    eval_set: str = typer.Option(..., "--set"),
    asr_name: str = typer.Option(..., "--asr"),
    name: str = typer.Option(...),
    model: str = "gemma4-e4b-qat",
    prompt: Path | None = typer.Option(None, help="A system prompt file instead of the product's."),
) -> None:
    from eval.finaltext.llm import model_path

    hyps = [
        json.loads(line)
        for line in (_results("asr", asr_name) / f"{eval_set}.jsonl").open(encoding="utf-8")
    ]
    cases = _results("llm", name)
    cases.mkdir(parents=True, exist_ok=True)
    case_file = cases / f"{eval_set}-input.jsonl"
    with case_file.open("w", encoding="utf-8") as fh:
        for h in hyps:
            fh.write(json.dumps({"id": h["id"], "input": h["hyp"]}, ensure_ascii=False) + "\n")
    cmd = [
        str(CLI),
        "llm-bench",
        "--gguf",
        str(model_path(model)),
        "--name",
        name,
        "--cases",
        str(case_file),
    ]
    cmd += [f"--server-arg={a}" for a in PRODUCT_SERVER_ARGS]
    if prompt is not None:
        cmd += ["--prompt", str(prompt.resolve())]
    subprocess.run(cmd, check=True)


class Tally:
    """Scores pooled over a group of dictations."""

    def __init__(self) -> None:
        self.n = 0
        self.exact = 0
        self.chars = 0.0
        self.ref_words = self.errors = 0
        self.marks: dict[str, list[int]] = defaultdict(lambda: [0, 0, 0])
        self.casing_checked = self.casing_wrong = 0
        self.spans: dict[str, list[int]] = defaultdict(lambda: [0, 0])

    def add(self, s: Score, span_kinds: list[tuple[str, bool]]) -> None:
        self.n += 1
        self.exact += s.exact
        self.chars += s.chars_to_fix
        self.ref_words += s.ref_words
        self.errors += s.substitutions + s.deletions + s.insertions
        for k, t in s.marks.items():
            for i in range(3):
                self.marks[k][i] += t[i]
        self.casing_checked += s.casing_checked
        self.casing_wrong += s.casing_wrong
        for kind, hit in span_kinds:
            self.spans[kind][0] += hit
            self.spans[kind][1] += 1

    def row(self) -> dict:
        out = {
            "n": self.n,
            "exact_pct": 100 * self.exact / max(1, self.n),
            "chars_to_fix_pct": 100 * self.chars / max(1, self.n),
            "word_error_pct": 100 * self.errors / max(1, self.ref_words),
            "casing_error_pct": 100 * self.casing_wrong / max(1, self.casing_checked),
        }
        for m in MARKS_REPORTED:
            if m in self.marks:
                p, r, f = prf(self.marks[m])
                out[f"mark {m!r}"] = {
                    "p": p,
                    "r": r,
                    "f1": f,
                    "support": self.marks[m][0] + self.marks[m][2],
                }
        for kind, (hit, total) in sorted(self.spans.items()):
            out[f"span {kind}"] = {"hit_pct": 100 * hit / max(1, total), "n": total}
        return out


def _rows(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def _latency(rows: list[dict], field: str, scale: float) -> dict[str, float]:
    """p50 and p95 of a per-row time, in milliseconds."""
    ms = sorted(r[field] * scale for r in rows if r.get(field) is not None)
    if not ms:
        return {}
    return {"p50_ms": ms[len(ms) // 2], "p95_ms": ms[min(len(ms) - 1, int(len(ms) * 0.95))]}


@app.command("score")
def score_cmd(
    eval_set: str = typer.Option(..., "--set"),
    asr_name: str = typer.Option(..., "--asr"),
    cleanup_names: list[str] = typer.Option([], "--cleanup"),
    gold: str = typer.Option(
        "fixed",
        help="fixed: the gold-fixed passages (eval.finaltext.goldfix), dropping those it "
        "dropped; screened: passages the screen did not flag; all: every passage as extracted.",
    ),
) -> None:
    rows = _manifest(eval_set)
    root = eval_dir() / "finaltext"
    if gold == "fixed":
        fixed_path = root / "passages-eval.jsonl"
        if not (root / "passages-eval.raw.jsonl").exists():
            raise typer.BadParameter(
                "eval passages are not gold-fixed yet: run eval.finaltext.goldfix"
            )
        fixed = {p["id"]: p["text"] for p in _rows(fixed_path)}
        before = len(rows)
        rows = [
            {**r, "gold": fixed[pid]} for r in rows if (pid := r["id"].rsplit("-", 1)[0]) in fixed
        ]
        console.print(
            f"{len(rows)} dictations scored against fixed gold; {before - len(rows)} dropped"
        )
    screen_path = root / "screen-eval.jsonl"
    clean_only = gold == "screened"
    if clean_only and screen_path.exists():
        # A passage is excluded only for gold problems the screen judges reliably enough; its
        # punctuation opinions are too noisy to exclude on (2026-10-10: 259 of 445 flags, many
        # wrong). Score without the screen too (--no-clean-only) to see what this changes.
        clean = {
            r["id"]
            for r in (json.loads(line) for line in screen_path.open(encoding="utf-8"))
            if not any(p["kind"] in SCREEN_EXCLUDES for p in r["problems"])
        }
        before = len(rows)
        rows = [r for r in rows if r["id"].rsplit("-", 1)[0] in clean]
        console.print(
            f"{len(rows)} dictations scored; {before - len(rows)} excluded by the gold screen"
        )
    asr_rows = _rows(_results("asr", asr_name) / f"{eval_set}.jsonl")
    systems = {"parakeet": {r["id"]: r["hyp"] for r in asr_rows}}
    # Each system's own time: the recognizer's per clip; a cleanup's per request (on top of it).
    latency = {"parakeet": _latency(asr_rows, "latency_s", 1000.0)}
    for name in cleanup_names:
        rows_out = _rows(_results("llm", name) / f"{eval_set}-input.jsonl")
        systems[name] = {r["id"]: r["output"] for r in rows_out}
        latency[name] = _latency(rows_out, "wall_ms", 1.0)

    summary: dict[str, dict[str, dict]] = {}
    for system, outs in systems.items():
        groups: dict[str, Tally] = defaultdict(Tally)
        for r in rows:
            if r["id"] not in outs:
                continue
            gold_spans = spans(r["gold"])
            s = score(r["gold"], outs[r["id"]], spans=[sp.text for sp in gold_spans])
            missed = set(s.missed_spans)
            span_kinds = [(sp.kind, sp.text not in missed) for sp in gold_spans]
            keys = ["all", f"domain={r['domain']}", f"kind={r['kind']}"]
            keys += [
                f"{r['kind']}:{k}={v}"
                for k, v in r.get("tags", {}).items()
                if k in ("scope", "cue", "command")
            ]
            for k in keys:
                groups[k].add(s, span_kinds)
        summary[system] = {k: t.row() for k, t in sorted(groups.items())}
        summary[system]["all"]["latency"] = latency[system]

    view = gold
    out = eval_dir() / "results" / "finaltext" / f"{eval_set}--{asr_name}--{view}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    _print(summary)
    console.print(f"written to {out}")


def _print(summary: dict[str, dict[str, dict]]) -> None:
    systems = list(summary)
    groups = sorted({g for s in summary.values() for g in s}, key=lambda g: (g != "all", g))
    t = Table(title="Final text: no edit needed % | characters to fix % | word error %")
    t.add_column("group")
    t.add_column("n", justify="right")
    for s in systems:
        t.add_column(s, justify="right")
    for g in groups:
        cells = []
        n = 0
        for s in systems:
            row = summary[s].get(g)
            if row is None:
                cells.append("-")
                continue
            n = row["n"]
            cells.append(
                f"{row['exact_pct']:.1f} | {row['chars_to_fix_pct']:.1f} | {row['word_error_pct']:.1f}"
            )
        t.add_row(g, str(n), *cells)
    console.print(t)

    m = Table(title="Marks, all dictations: F1 (precision / recall)")
    m.add_column("mark")
    m.add_column("support", justify="right")
    for s in systems:
        m.add_column(s, justify="right")
    for mark in MARKS_REPORTED:
        key = f"mark {mark!r}"
        rows = [summary[s]["all"].get(key) for s in systems]
        if not any(rows):
            continue
        support = max(r["support"] for r in rows if r)
        m.add_row(
            MARK_NAMES.get(mark, mark),
            str(support),
            *[f"{r['f1']:.2f} ({r['p']:.2f}/{r['r']:.2f})" if r else "-" for r in rows],
        )
    console.print(m)

    sp = Table(title="Spans that must come out exactly, all dictations: hit %")
    sp.add_column("kind")
    for s in systems:
        sp.add_column(s, justify="right")
    kinds = sorted({k for s in systems for k in summary[s]["all"] if k.startswith("span ")})
    for k in kinds:
        sp.add_row(
            f"{k[5:]} (n={summary[systems[0]]['all'][k]['n']})",
            *[f"{summary[s]['all'][k]['hit_pct']:.1f}" for s in systems],
        )
    console.print(sp)

    lat = Table(title="Time per dictation (recognizer per clip; cleanup per request, on top of it)")
    lat.add_column("system")
    lat.add_column("p50 ms", justify="right")
    lat.add_column("p95 ms", justify="right")
    for s in systems:
        t = summary[s]["all"].get("latency") or {}
        lat.add_row(s, f"{t.get('p50_ms', 0):.0f}", f"{t.get('p95_ms', 0):.0f}")
    console.print(lat)


if __name__ == "__main__":
    app()
