"""A user's dictionary in the eval: phrase boosting in the recognizer (crates/utter-asr/src/boost.rs)
and dictionary terms in cleanup.

- `make` samples terms a user would put in a dictionary (text.terms) from an eval set's golds into
  a dictionary file. The evals' dictionaries are kept with
  the code (eval/dictionaries/<set>-<size>.txt): they are part of the protocol.
- `terms` compares runs on the dictionary's terms: recall (a term in the gold written exactly,
  case and all), false insertions (a term written more often than the gold has it), recall of
  the set's other terms (a control: boosting must not pull them toward dictionary words), and
  clips whose text differs from the first run. A run is a recognizer run (`asr:<run>`) or a
  cleanup run (`llm:<run>`).
"""

from __future__ import annotations

import random
from pathlib import Path

from utter_tools.eval.sets import asr_path, llm_path, manifest, read_jsonl
from utter_tools.text.spans import spans
from utter_tools.text.terms import candidate_terms, lowercase_counts, tokens

DICTIONARIES = Path(__file__).parent / "dictionaries"


def make(eval_set: str, size: int = 300, seed: int = 0, names: bool = False) -> Path:
    """Writes `size` distinct terms drawn from the set's golds (text.terms), one per line;
    `names` adds proper names."""
    golds = [r["final_text"] for r in manifest(eval_set)]
    lower = lowercase_counts(golds)
    terms = sorted({t for g in golds for t in candidate_terms(g, lower, names)})
    picked = sorted(random.Random(seed).sample(terms, min(size, len(terms))))
    suffix = f"-s{seed}" if seed else ""
    out = DICTIONARIES / f"{eval_set}-{len(picked)}{suffix}.txt"
    out.write_text("\n".join(picked) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(picked)} of {len(terms)} terms -> {out}")
    return out


def read(path: Path) -> list[str]:
    return [t.strip() for t in path.read_text(encoding="utf-8").splitlines() if t.strip()]


def _tally(gold: str, hyp: str, terms: set[str]) -> tuple[int, int, int]:
    """(term occurrences in the gold, of those written exactly, dictionary terms inserted)."""
    g, h = tokens(gold), tokens(hyp)
    present = sum(g[t] for t in terms)
    hit = sum(min(g[t], h[t]) for t in terms)
    inserted = sum(max(0, h[t] - g[t]) for t in terms)
    return present, hit, inserted


def _outputs(run: str, eval_set: str) -> dict[str, str]:
    kind, name = run.split(":", 1)
    if kind == "asr":
        return {r["id"]: r["hyp"] for r in read_jsonl(asr_path(name, eval_set))}
    if kind == "llm":
        return {r["id"]: r["output"] for r in read_jsonl(llm_path(name, eval_set))}
    raise ValueError(f"{run}: a run is asr:<name> or llm:<name>")


def terms(eval_set: str, dictionary: Path, runs: list[str]) -> None:
    words = set(read(dictionary))
    gold = {r["id"]: r["final_text"] for r in manifest(eval_set)}
    others = {s.text for g in gold.values() for s in spans(g) if s.kind == "term"} - words
    print(f"{eval_set}: {len(gold)} clips, {len(words)} dictionary terms, {len(others)} others")
    print(f"{'run':<24} {'recall':>20} {'inserted':>9} {'others':>20} {'changed':>8}")
    base: dict[str, str] | None = None
    for run in runs:
        outs = _outputs(run, eval_set)
        present = hit = inserted = o_present = o_hit = 0
        for cid, g in gold.items():
            p, h, i = _tally(g, outs.get(cid, ""), words)
            present, hit, inserted = present + p, hit + h, inserted + i
            p, h, _ = _tally(g, outs.get(cid, ""), others)
            o_present, o_hit = o_present + p, o_hit + h
        changed = sum(outs.get(c) != base.get(c) for c in gold) if base else 0
        base = base or outs
        recall = f"{100 * hit / max(1, present):.1f}% ({hit}/{present})"
        control = f"{100 * o_hit / max(1, o_present):.1f}% ({o_hit}/{o_present})"
        print(f"{run:<24} {recall:>20} {inserted:>9} {control:>20} {changed:>8}")
