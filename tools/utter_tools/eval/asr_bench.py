"""The speech-recognition accuracy bench: each candidate model on each eval set.

For every (model, set) pair it writes `<eval_dir>/results/asr/<model>/<set>.jsonl` (one row
per clip: id, ref, hyp, detected language, latency, duration) and prints WER, the language
detection error rate, and latency per clip at batch 1. A pair already complete is skipped, so
the bench resumes after an interruption; `--redo` forces it. `report` re-scores everything
on disk into one table.

The latency here is the model's own runtime in Python (Transformers or onnx-asr), so it ranks
models but is not Utter's latency: the Rust core's runtime is measured by `utter-cli bench`.

The sets are the public test sets public_sets.py fetches, not the cleanup eval's.

Run: utter-eval bench run [--models a,b] [--sets x,y] [--limit N]; utter-eval bench report
"""

from __future__ import annotations

import gc
import json
import statistics
import time
from pathlib import Path

import jiwer
import numpy as np
import soundfile as sf
from rich.console import Console
from rich.table import Table

from utter_tools.config import eval_dir, results_dir
from utter_tools.eval.asr_models import AsrModel, registry
from utter_tools.eval.public_sets import SETS
from utter_tools.text.normalize import normalize

WARMUP_CLIPS = 3

console = Console()


PUBLIC_SETS = [s.name for s in SETS]


def _asr_dir() -> Path:
    return results_dir() / "asr"


def available_sets() -> dict[str, list[dict]]:
    """The public sets fetched so far, by name."""
    sets = {}
    for name in PUBLIC_SETS:
        manifest = eval_dir() / name / "manifest.jsonl"
        if manifest.is_file():
            rows = [json.loads(line) for line in manifest.open(encoding="utf-8")]
            if rows:
                sets[name] = rows
    return sets


def run_pair(
    model: AsrModel, set_name: str, rows: list[dict], limit: int | None, redo: bool
) -> None:
    out = _asr_dir() / model.name / f"{set_name}.jsonl"
    rows = rows[:limit] if limit else rows
    if out.exists() and not redo and sum(1 for _ in out.open(encoding="utf-8")) >= len(rows):
        return
    out.parent.mkdir(parents=True, exist_ok=True)
    folder = eval_dir() / set_name
    clips = [(r, sf.read(folder / r["audio"], dtype="float32")[0]) for r in rows]
    for _, audio in clips[:WARMUP_CLIPS]:  # first calls pay for kernel compilation and caches
        model.transcribe(audio, clips[0][0]["lang"])
    model.sync()
    tmp = out.with_suffix(".jsonl.part")
    with (
        tmp.open("w", encoding="utf-8", newline="\n") as f,
        console.status(f"{model.name} on {set_name}") as status,
    ):
        for i, (row, audio) in enumerate(clips):
            start = time.perf_counter()
            result = model.transcribe(audio, row["lang"])
            model.sync()
            latency = time.perf_counter() - start
            f.write(
                json.dumps(
                    {
                        "id": row["id"],
                        "lang": row["lang"],
                        "ref": row["text"],
                        "hyp": result.text,
                        "detected": result.language,
                        "latency_s": round(latency, 4),
                        "duration_s": row["duration_s"],
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
            status.update(f"{model.name} on {set_name}: {i + 1}/{len(clips)}")
    tmp.replace(out)


def run(models: list[str], sets: list[str], limit: int = 0, redo: bool = False) -> None:
    """Every chosen model (default: all) on every chosen fetched set (default: all) it covers."""
    reg = registry()
    chosen = [reg[m] for m in models] if models else list(reg.values())
    all_sets = available_sets()
    set_names = sets or list(all_sets)
    for model in chosen:
        pairs = [s for s in set_names if all_sets[s][0]["lang"] in model.langs]
        if not pairs:
            continue
        console.print(f"[bold]{model.name}[/bold]: loading")
        try:
            model.load()
        except Exception as e:  # noqa: BLE001 -- one model failing to load must not end the bench
            console.print(f"[red]{model.name}: failed to load: {type(e).__name__}: {e}[/red]")
            continue
        (_asr_dir() / model.name).mkdir(parents=True, exist_ok=True)
        notes = {**model.notes, "auto_language": model.auto_language}
        (_asr_dir() / model.name / "notes.json").write_text(json.dumps(notes, indent=2))
        for set_name in pairs:
            try:
                run_pair(model, set_name, all_sets[set_name], limit or None, redo)
            except Exception as e:  # noqa: BLE001
                console.print(f"[red]{model.name} on {set_name}: {type(e).__name__}: {e}[/red]")
        del model
        gc.collect()
        try:
            import torch

            torch.cuda.empty_cache()
        except ImportError:
            pass
    report()


def score(path: Path) -> dict:
    rows = [json.loads(line) for line in path.open(encoding="utf-8")]
    lang = rows[0]["lang"]
    refs = [normalize(r["ref"], lang) for r in rows]
    hyps = [normalize(r["hyp"], lang) for r in rows]
    keep = [i for i, ref in enumerate(refs) if ref]
    wer = jiwer.wer([refs[i] for i in keep], [hyps[i] for i in keep])
    # For a model that reports languages, a clip it gave no language for is a miss too: the
    # tag was absent or unreadable, so Utter could not route on it.
    reports_language = any(r["detected"] for r in rows)
    lid_err = (sum(r["detected"] != lang for r in rows) / len(rows)) if reports_language else None
    lat = sorted(r["latency_s"] for r in rows)
    audio_s = sum(r["duration_s"] for r in rows)
    return {
        "n": len(rows),
        "wer": wer,
        "lid_err": lid_err,
        "p50": statistics.median(lat),
        "p95": lat[min(len(lat) - 1, int(0.95 * len(lat)))],
        "rtfx": audio_s / sum(lat),
    }


def report() -> None:
    """WER of every model on every public set it has results for."""
    scores: dict[str, dict[str, dict]] = {}
    for path in sorted(_asr_dir().glob("*/*.jsonl")):
        if path.stem in PUBLIC_SETS:
            scores.setdefault(path.parent.name, {})[path.stem] = score(path)
    if not scores:
        console.print("No results yet.")
        return
    set_names = sorted({s for m in scores.values() for s in m})
    table = Table(
        title="WER % (lower is better); last columns: per-language mean WER, latency at batch 1"
    )
    table.add_column("model")
    for s in set_names:
        table.add_column(s, justify="right")
    for col in ("mean en", "mean vi", "LID err", "p50 ms", "p95 ms"):
        table.add_column(col, justify="right")
    for model, by_set in sorted(scores.items()):
        cells = [f"{by_set[s]['wer'] * 100:.1f}" if s in by_set else "" for s in set_names]
        means = []
        for lang in ("en", "vi"):
            vals = [v["wer"] for s, v in by_set.items() if s.startswith(lang + "-")]
            means.append(f"{np.mean(vals) * 100:.1f}" if vals else "")
        lids = [v["lid_err"] for v in by_set.values() if v["lid_err"] is not None]
        lats = [v["p50"] for v in by_set.values()]
        p95s = [v["p95"] for v in by_set.values()]
        table.add_row(
            model,
            *cells,
            *means,
            f"{np.mean(lids) * 100:.1f}%" if lids else "",
            f"{np.median(lats) * 1000:.0f}",
            f"{np.median(p95s) * 1000:.0f}",
        )
    console.print(table)


def _clip_errors(model: str, lang: str) -> dict[tuple[str, str], tuple[int, int]]:
    """(set, clip id) -> (word errors, reference words) for every clip of `lang` the model did."""
    out = {}
    for path in sorted((_asr_dir() / model).glob(f"{lang}-*.jsonl")):
        for line in path.open(encoding="utf-8"):
            r = json.loads(line)
            ref, hyp = normalize(r["ref"], lang), normalize(r["hyp"], lang)
            if not ref:
                continue
            m = jiwer.process_words(ref, hyp)
            out[(path.stem, r["id"])] = (
                m.substitutions + m.deletions + m.insertions,
                len(ref.split()),
            )
    return out


def compare(a: str, b: str, lang: str = "en", resamples: int = 10_000) -> None:
    """Paired bootstrap of pooled WER, A minus B, over the clips both models did: is the gap
    bigger than what the choice of clips alone would produce?"""
    ea, eb = _clip_errors(a, lang), _clip_errors(b, lang)
    keys = sorted(ea.keys() & eb.keys())
    if not keys:
        console.print("No clips in common.")
        return
    errs_a = np.array([ea[k][0] for k in keys], dtype=float)
    errs_b = np.array([eb[k][0] for k in keys], dtype=float)
    words = np.array([ea[k][1] for k in keys], dtype=float)
    rng = np.random.default_rng(0)
    idx = rng.integers(0, len(keys), size=(resamples, len(keys)))
    diffs = (errs_a[idx].sum(1) - errs_b[idx].sum(1)) / words[idx].sum(1) * 100
    point = (errs_a.sum() - errs_b.sum()) / words.sum() * 100
    lo, hi = np.percentile(diffs, [2.5, 97.5])
    console.print(
        f"{a} minus {b}, {lang}, {len(keys)} clips: {point:+.2f} WER points "
        f"(95% CI {lo:+.2f} to {hi:+.2f}); {a} worse in {np.mean(diffs > 0) * 100:.1f}% of resamples"
    )
