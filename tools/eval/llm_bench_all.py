"""Bench every cleanup LLM candidate through Utter's own runtime (`utter-cli llm-bench`) on the
curated and real-speech cleanup cases. Candidates already benched are skipped (`--redo` to
rerun). Score with `eval.cleanup_score report`.

Run: uv run python -m eval.llm_bench_all [NAMES...] [--server-arg ARG ...] [--suffix S]
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import typer

from eval.llm_fetch import candidates, local_path
from eval.paths import eval_dir

REPO = Path(__file__).resolve().parents[2]
CLI = (
    REPO
    / "target"
    / "release"
    / ("utter-cli.exe" if (REPO / "target" / "release" / "utter-cli.exe").exists() else "utter-cli")
)
CASES = [
    Path(__file__).with_name("cleanup_cases.jsonl"),
    eval_dir() / "cleanup" / "cleanup_real.jsonl",
]


def main(
    names: list[str] = typer.Argument(None),
    server_arg: list[str] = typer.Option([], help="Passed to llama-server, repeatable."),
    suffix: str = typer.Option("", help="Appended to result names, e.g. '-ngram'."),
    redo: bool = typer.Option(False),
) -> None:
    for c in candidates():
        if names and c["name"] not in names:
            continue
        name = c["name"] + suffix
        done = eval_dir() / "results" / "llm" / name / "cleanup_real.jsonl"
        if done.exists() and not redo:
            print(f"{name}: done, skipped", flush=True)
            continue
        cmd = [str(CLI), "llm-bench", "--gguf", str(local_path(c)), "--name", name]
        for case in CASES:
            cmd += ["--cases", str(case)]
        for a in server_arg:
            cmd.append(f"--server-arg={a}")
        print(f"{name}: running", flush=True)
        # check=False: a candidate that fails to load is reported and the next one runs.
        result = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", check=False)
        lines = [ln for ln in (result.stdout + result.stderr).splitlines() if "INFO" not in ln]
        print("\n".join(lines[-4:]), flush=True)


if __name__ == "__main__":
    typer.run(main)
