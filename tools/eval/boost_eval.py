"""Measures phrase boosting (crates/utter-asr/src/boost.rs): does a dictionary make the
recognizer write its terms, and what does it cost elsewhere?

1. `dictionary` samples terms (spans.py's "term" kind: GitHub, TF32, LTS) from a final-text
   set's golds into a phrases file, as a user's dictionary would hold them.
2. Transcribe with and without it: `utter-cli bench --sets SET --name NAME --phrases FILE`.
3. `terms` compares the runs on the dictionary's terms: recall (a term in the gold written
   exactly, case and all) and false insertions (a dictionary term written more often than the
   gold has it). `asr_bench.py compare` gives the word error rate on other sets.

Run from tools/: uv run python -m eval.boost_eval dictionary --set en-finaltext-kokoro --size 300
"""

from __future__ import annotations

import json
import random
import re
from collections import Counter
from itertools import pairwise
from pathlib import Path

import typer

from eval.finaltext.spans import _EDGE, spans
from eval.paths import eval_dir

app = typer.Typer(add_completion=False)
_WORD = re.compile(r"[A-Za-z][A-Za-z0-9]*(?:['-][A-Za-z0-9]+)*")


def _manifest(eval_set: str) -> list[dict]:
    path = eval_dir() / eval_set / "manifest.jsonl"
    return [json.loads(line) for line in path.open(encoding="utf-8")]


def tokens(text: str) -> Counter[str]:
    """Words as written, with surrounding punctuation stripped."""
    return Counter(t for t in (w.strip(_EDGE) for w in text.split()) if t)


@app.command()
def dictionary(
    eval_set: str = typer.Option(..., "--set"),
    size: int = 300,
    seed: int = 0,
    out: Path | None = None,
    names: bool = typer.Option(
        False,
        help="Also proper names (Safeway, Smithson): capitalized mid-sentence and never written "
        "in lower case in the set.",
    ),
) -> None:
    """Writes `size` distinct terms from the set's golds, one per line, as a user's dictionary
    would hold them: words (MySQL, TF32, Kerr-Newman), not paths or sizes ("/usr/share",
    "120GB"), and not everyday words in capitals ("KEY", "ON": emphasis or SQL)."""
    golds = [r["gold"] for r in _manifest(eval_set)]
    lower = Counter(t for g in golds for t in tokens(g) if t.islower())

    def wanted(term: str) -> bool:
        return bool(_WORD.fullmatch(term)) and lower[term.lower()] < 3

    found = {s.text for g in golds for s in spans(g) if s.kind == "term" and wanted(s.text)}
    if names:
        for g in golds:
            words = g.split()
            for prev, raw in pairwise(words):
                tok = raw.strip(_EDGE)
                if (
                    not prev.endswith((".", "!", "?", ":"))
                    and tok[:1].isupper()
                    and tok[1:].islower()
                    and len(tok) > 2
                    and _WORD.fullmatch(tok)
                    and lower[tok.lower()] == 0
                ):
                    found.add(tok)
    terms = sorted(found)
    picked = sorted(random.Random(seed).sample(terms, min(size, len(terms))))
    # Kept with the code (tools/eval/dictionaries): the evals' dictionaries are part of the protocol.
    out = (
        out or Path(__file__).parent / "dictionaries" / f"{eval_set}-terms{len(picked)}-s{seed}.txt"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(picked) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(picked)} of {len(terms)} terms -> {out}")


def tally(gold: str, hyp: str, terms: set[str]) -> tuple[int, int, int]:
    """(term occurrences in the gold, of those written exactly, dictionary terms inserted)."""
    g, h = tokens(gold), tokens(hyp)
    present = sum(g[t] for t in terms)
    hit = sum(min(g[t], h[t]) for t in terms)
    inserted = sum(max(0, h[t] - g[t]) for t in terms)
    return present, hit, inserted


@app.command()
def terms(
    eval_set: str = typer.Option(..., "--set"),
    phrases: Path = typer.Option(...),
    runs: list[str] = typer.Option(..., "--run", help="ASR result names (results/asr/<name>/)."),
) -> None:
    """Per run: recall and false insertions of the dictionary's terms; recall of the set's other
    terms (a control: boosting must not pull them toward dictionary words); word error rate
    against what was spoken; and clips whose text differs from the first run."""
    from eval.asr_bench import score

    words = {line.strip() for line in phrases.read_text(encoding="utf-8").splitlines()}
    words.discard("")
    # ASR sets have no written gold; what was said stands in (for insertions and WER).
    gold = {r["id"]: r.get("gold", r["text"]) for r in _manifest(eval_set)}
    others = {s.text for g in gold.values() for s in spans(g) if s.kind == "term"} - words
    print(f"{eval_set}: {len(gold)} clips, {len(words)} dictionary terms, {len(others)} others")
    print(f"{'run':<12} {'recall':>20} {'inserted':>9} {'others':>20} {'WER':>7} {'changed':>8}")
    base: dict[str, str] | None = None
    for name in runs:
        path = eval_dir() / "results" / "asr" / name / f"{eval_set}.jsonl"
        hyps = {r["id"]: r["hyp"] for r in map(json.loads, path.open(encoding="utf-8"))}
        present = hit = inserted = o_present = o_hit = 0
        for cid, g in gold.items():
            p, h, i = tally(g, hyps.get(cid, ""), words)
            present, hit, inserted = present + p, hit + h, inserted + i
            p, h, _ = tally(g, hyps.get(cid, ""), others)
            o_present, o_hit = o_present + p, o_hit + h
        changed = sum(hyps.get(c) != base.get(c) for c in gold) if base else 0
        base = base or hyps
        recall = f"{100 * hit / max(1, present):.1f}% ({hit}/{present})"
        control = f"{100 * o_hit / max(1, o_present):.1f}% ({o_hit}/{o_present})"
        wer = 100 * score(path)["wer"]
        print(f"{name:<12} {recall:>20} {inserted:>9} {control:>20} {wer:>6.2f}% {changed:>8}")


if __name__ == "__main__":
    app()
