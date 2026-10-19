"""Scores `utter-cli long-bench` results: each (live window, key-up window) pair's text against
the reference final text, and against the whole-dictation cleanup (the largest windows) where
that ran; with request times while speaking and the wait at key up.

    uv run python -m utter_tools.long_score <name>      (results/long/<name>/<window>.jsonl)
"""

from __future__ import annotations

import json
import statistics
import sys

import jiwer

from utter_tools import common
from utter_tools.bench import normalize


def _rows(path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def _pct(values, q):
    values = sorted(values)
    return values[round(q * (len(values) - 1))]


def main(name: str) -> None:
    folder = common.eval_dir() / "results" / "long" / name
    runs = {int(p.stem): _rows(p) for p in folder.glob("*.jsonl")}
    if not runs:
        sys.exit(f"no results in {folder}")
    # Only the dictations every window size has finished (a run may still be going).
    ids = set.intersection(*({r["dictation"] for r in rows} for rows in runs.values()))
    runs = {w: sorted((r for r in rows if r["dictation"] in ids), key=lambda r: r["dictation"]) for w, rows in runs.items()}
    biggest = max(runs)
    top_final = max(runs[biggest][0]["finals"], key=int)
    whole = {r["dictation"]: r["finals"][top_final]["text"] for r in runs[biggest]}
    words = [r["words"] for r in runs[biggest]]
    print(f"{name}: {len(ids)} dictations of {min(words)}-{max(words)} words")
    ref = [normalize(r["reference"], "en") for r in runs[biggest]]
    raw = [normalize(r["raw"], "en") for r in runs[biggest]]
    print(f"raw speech vs reference: {jiwer.wer(ref, raw):.2%}")
    print(f"{'live':>7} {'key up':>7} {'WER':>7} {'vs whole':>9} {'live p50':>9} {'live p95':>9} {'key up p50':>11} {'max':>6}")
    for window in sorted(runs):
        rows = runs[window]
        live = [m for r in rows for m in r["ms"]]
        for final in sorted(rows[0]["finals"], key=int):
            hyp = [normalize(r["finals"][final]["text"], "en") for r in rows]
            vs_whole = jiwer.wer([normalize(whole[r["dictation"]], "en") for r in rows], hyp)
            waits = [r["finals"][final]["ms"] for r in rows]
            print(
                f"{window:>7} {final:>7} {jiwer.wer(ref, hyp):>7.2%} {vs_whole:>9.2%} {statistics.median(live):>9.0f}"
                f" {_pct(live, 0.95):>9.0f} {statistics.median(waits):>11.0f} {max(waits):>6.0f}"
            )


if __name__ == "__main__":
    main(sys.argv[1])
