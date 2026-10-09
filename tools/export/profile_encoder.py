"""Where does the encoder's time go on a clip length it has not seen before?

Runs the exported encoder with onnxruntime's profiler on lengths A, A, B, B, A and sums the
node time by operator type for the first run of a new length and for a repeat. A diagnostic,
not part of the bench.

Run: uv run python -m export.profile_encoder [--name parakeet-ultra]
"""

from __future__ import annotations

import json
from collections import defaultdict

import numpy as np
import onnxruntime as ort
import typer
from rich.console import Console
from rich.table import Table

from eval.paths import data_dir

console = Console()


def main(name: str = "parakeet-ultra", frames_a: int = 300, frames_b: int = 437) -> None:
    ort.preload_dlls()
    so = ort.SessionOptions()
    so.enable_profiling = True
    so.profile_file_prefix = str(data_dir() / "profile-encoder")
    cuda = ("CUDAExecutionProvider", {"cudnn_conv_algo_search": "HEURISTIC", "use_tf32": "1"})
    sess = ort.InferenceSession(
        str(data_dir() / "models" / name / "encoder.onnx"), so, providers=[cuda]
    )
    rng = np.random.default_rng(0)
    order = [frames_a, frames_a, frames_b, frames_b, frames_a]
    for n in order:
        sess.run(None, {"features": rng.standard_normal((1, n, 128)).astype(np.float32)})
    with open(sess.end_profiling(), encoding="utf-8") as f:
        trace = json.load(f)

    # Group node events by the model_run they fall inside.
    run_events = [
        ev for ev in trace if ev.get("cat") == "Session" and ev.get("name") == "model_run"
    ]
    totals = [ev["dur"] / 1e3 for ev in run_events]
    run_spans = [(ev["ts"], ev["ts"] + ev["dur"]) for ev in run_events]
    runs: list[dict[str, float]] = [defaultdict(float) for _ in run_spans]
    for ev in trace:
        if ev.get("cat") != "Node" or not ev.get("name", "").endswith("_kernel_time"):
            continue
        for i, (s, e) in enumerate(run_spans):
            if s <= ev["ts"] <= e:
                runs[i][ev.get("args", {}).get("op_name", "?")] += ev["dur"] / 1e3
                break

    table = Table(title="encoder node time by operator (ms)")
    table.add_column("op")
    for i, n in enumerate(order):
        table.add_column(f"run {i + 1}: {n} frames", justify="right")
    ops = sorted({op for r in runs for op in r}, key=lambda op: -max(r.get(op, 0) for r in runs))
    for op in ops[:15]:
        table.add_row(op, *[f"{r.get(op, 0):.1f}" for r in runs])
    table.add_row("[bold]nodes total[/bold]", *[f"{sum(r.values()):.1f}" for r in runs])
    table.add_row("[bold]model_run[/bold]", *[f"{t:.1f}" for t in totals])
    console.print(table)


if __name__ == "__main__":
    typer.run(main)
