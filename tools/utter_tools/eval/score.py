"""Strict scores of each system on an eval set against the gold (the dataset row's final text):
no edit needed, characters to fix, word errors, every punctuation mark's precision and recall,
casing, and the spans that must come out exactly (text.spans), overall and by domain, category
and annotation. The recognizer alone is scored too, as the floor.

Written to `results/score/<set>--<asr run>.json` and printed as tables.
"""

from __future__ import annotations

import json
from collections import defaultdict

from rich.console import Console
from rich.table import Table

from utter_tools import config, schema
from utter_tools.eval.sets import asr_path, llm_path, manifest, read_jsonl
from utter_tools.text.score import Score, prf, score
from utter_tools.text.spans import spans

console = Console(width=160)

MARKS_REPORTED = [".", ",", "?", "!", ":", ";", "—", "(", ")", '"', "'", "\n", "\n\n"]
MARK_NAMES = {"—": "dash", "\n": "line break", "\n\n": "paragraph", '"': "quote", "'": "apostrophe"}
# Tags that split a kind's results (spoken.py's corrections and commands).
SPLIT_TAGS = ("scope", "cue", "command")


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


def _latency(rows: list[dict], field: str, scale: float) -> dict[str, float]:
    """p50 and p95 of a per-row time, in milliseconds."""
    ms = sorted(r[field] * scale for r in rows if r.get(field) is not None)
    if not ms:
        return {}
    return {"p50_ms": ms[len(ms) // 2], "p95_ms": ms[min(len(ms) - 1, int(len(ms) * 0.95))]}


def summarize(eval_set: str, asr_run: str, runs: list[str]) -> dict[str, dict[str, dict]]:
    """System -> group -> scores, for the recognizer run alone and each cleanup run."""
    rows = manifest(eval_set)
    asr_rows = read_jsonl(asr_path(asr_run, eval_set))
    systems = {asr_run: {r["id"]: r["hyp"] for r in asr_rows}}
    # Each system's own time: the recognizer's per clip; a cleanup's per request (on top of it).
    latency = {asr_run: _latency(asr_rows, "latency_s", 1000.0)}
    for run in runs:
        outs = read_jsonl(llm_path(run, eval_set))
        systems[run] = {r["id"]: r["output"] for r in outs}
        latency[run] = _latency(outs, "wall_ms", 1.0)
    summary: dict[str, dict[str, dict]] = {}
    for system, outs in systems.items():
        groups: dict[str, Tally] = defaultdict(Tally)
        for r in rows:
            if r["id"] not in outs:
                continue
            gold_spans = spans(r["final_text"])
            s = score(r["final_text"], outs[r["id"]], spans=[sp.text for sp in gold_spans])
            missed = set(s.missed_spans)
            span_kinds = [(sp.kind, sp.text not in missed) for sp in gold_spans]
            tags = schema.from_annotations(r.get("annotations", []))
            keys = ["all", f"domain={r['domain']}", f"category={r['category']}"]
            keys += [f"{r['category']}:{k}={v}" for k, v in tags.items() if k in SPLIT_TAGS]
            for k in keys:
                groups[k].add(s, span_kinds)
        summary[system] = {k: t.row() for k, t in sorted(groups.items())}
        summary[system]["all"]["latency"] = latency[system]
    return summary


def run(eval_set: str, asr_run: str, runs: list[str]) -> dict[str, dict[str, dict]]:
    summary = summarize(eval_set, asr_run, runs)
    out = config.results_dir() / "score" / f"{eval_set}--{asr_run}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    _print(summary)
    console.print(f"written to {out}")
    return summary


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
