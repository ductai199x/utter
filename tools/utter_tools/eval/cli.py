"""utter-eval: grade cleanup models on the datasets' test splits, and speech models on public sets.

utter-eval run --model quality-v2=PATH.gguf [--model ...] [--llm-url URL]
    the whole standard eval (plan.py) for each model: sets, recognizer, cleanup, strict
    scores, ready to send, and the comparison report;
utter-eval sets | asr | clean | score | judge | report
    the same stages one at a time (each resumes where it stopped);
utter-eval dictionary make | terms
    eval dictionaries, and how a dictionary's terms come out;
utter-eval bench fetch | run | report | compare
    speech models on public test sets (word error rate).
"""

from __future__ import annotations

import os
from pathlib import Path

import typer

from utter_tools import config, llm
from utter_tools.eval import (
    asr_bench,
    dictionary,
    judge,
    plan,
    public_sets,
    report,
    runs,
    score,
    sets,
)

app = typer.Typer(
    add_completion=False,
    no_args_is_help=True,
    help="Grade cleanup models on the datasets' test splits, and speech models on public sets.",
)
dictionary_app = typer.Typer(no_args_is_help=True, help="Eval dictionaries and their terms.")
bench_app = typer.Typer(no_args_is_help=True, help="Speech models on public test sets.")
app.add_typer(dictionary_app, name="dictionary")
app.add_typer(bench_app, name="bench")

RUNS = typer.Option(..., "--run", help="Cleanup runs (model names), repeatable.")
LLM_URL = typer.Option("", help="An OpenAI-style grader (vLLM on a GPU cluster); else the local 26B.")
GRADER = typer.Option("", help="The grader's name in results (default: vllm-26b or local-26b).")
WORKERS = typer.Option(8, help="Grading requests at once (64 suits vLLM).")


def _grader(url: str, name: str) -> str:
    return name or ("vllm-26b" if url else "local-26b")


def _models(specs: list[str]) -> dict[str, Path]:
    out = {}
    for spec in specs:
        name, sep, path = spec.partition("=")
        if not sep or not Path(path).is_file():
            raise typer.BadParameter(f"{spec}: expected NAME=PATH to a .gguf file")
        out[name] = Path(path)
    return out


def _items() -> list[plan.Item]:
    speech = sorted(
        p.parent.name
        for p in config.eval_dir().glob("*/source.json")
        if p.parent.name != sets.TEXT_SET
    )
    if not speech:
        raise typer.BadParameter("no eval sets yet: utter-eval sets")
    return plan.items(speech)


@app.command("sets")
def sets_cmd(
    text_ref: str = typer.Option(None, help="dictation-to-text revision (default: main)."),
    speech_ref: str = typer.Option(None, help="dictation-speech revision (default: main)."),
) -> None:
    """Make the eval sets from the datasets' test splits."""
    for repo, rev in sets.make(text_ref, speech_ref).items():
        print(f"{repo} @ {rev}")


@app.command()
def asr() -> None:
    """Utter's recognizer on every speech set, and with each set's dictionary boosted."""
    for item in _items():
        if item.asr_run != sets.DICTATED:
            runs.asr(item.eval_set, item.asr_run, item.dictionary)


@app.command()
def clean(
    model: list[str] = typer.Option(..., help="NAME=PATH.gguf, repeatable."),
    gpu: str = typer.Option("", help="CUDA device to use (CUDA_VISIBLE_DEVICES)."),
) -> None:
    """Each model's cleanup on every set."""
    if gpu:
        os.environ["CUDA_VISIBLE_DEVICES"] = gpu
    for name, gguf in _models(model).items():
        for item in _items():
            runs.clean(item.eval_set, name + item.suffix, gguf, item.asr_run, item.dictionary)


@app.command("score")
def score_cmd(run: list[str] = RUNS) -> None:
    """Strict scores of the runs on every set."""
    for item in _items():
        score.run(item.eval_set, item.asr_run, [r + item.suffix for r in run])


@app.command("judge")
def judge_cmd(
    run: list[str] = RUNS, llm_url: str = LLM_URL, grader: str = GRADER, workers: int = WORKERS
) -> None:
    """Ready to send: grade the runs' misses on the graded sets."""
    with llm.connect(llm_url, slots=workers) as chat:
        for item in _items():
            if item.graded:
                for r in run:
                    judge.grade(
                        item.eval_set, r + item.suffix, chat, _grader(llm_url, grader), workers
                    )


@app.command("report")
def report_cmd(run: list[str] = RUNS, llm_url: str = LLM_URL, grader: str = GRADER) -> None:
    """One table comparing the runs over every set."""
    report.write(_items(), run, _grader(llm_url, grader))


@app.command("run")
def run_cmd(
    model: list[str] = typer.Option(..., help="NAME=PATH.gguf, repeatable."),
    llm_url: str = LLM_URL,
    grader: str = GRADER,
    workers: int = WORKERS,
    text_ref: str = typer.Option(None, help="dictation-to-text revision (default: main)."),
    speech_ref: str = typer.Option(None, help="dictation-speech revision (default: main)."),
) -> None:
    """The whole standard eval for each model, then the comparison report."""
    names = list(_models(model))
    sets_cmd(text_ref, speech_ref)
    asr()
    clean(model, "")
    score_cmd(names)
    judge_cmd(names, llm_url, grader, workers)
    report_cmd(names, llm_url, grader)


@dictionary_app.command("make")
def dictionary_make(
    eval_set: str = typer.Option(..., "--set"),
    size: int = 300,
    seed: int = 0,
    names: bool = typer.Option(False, help="Also proper names."),
) -> None:
    """Sample a dictionary from a set's golds."""
    dictionary.make(eval_set, size, seed, names)


@dictionary_app.command("terms")
def dictionary_terms(
    eval_set: str = typer.Option(..., "--set"),
    dictionary_file: Path = typer.Option(..., "--dictionary"),
    run: list[str] = typer.Option(..., "--run", help="asr:<run> or llm:<run>, repeatable."),
) -> None:
    """How the dictionary's terms come out in each run."""
    dictionary.terms(eval_set, dictionary_file, run)


@bench_app.command("fetch")
def bench_fetch(names: list[str] = typer.Argument(None), per_set: int = 300) -> None:
    """Fetch public test sets (a fixed sample of each)."""
    public_sets.fetch_all(names or [], per_set)


@bench_app.command("run")
def bench_run(
    models: list[str] = typer.Option([], "--model", help="Default: every candidate."),
    set_names: list[str] = typer.Option([], "--set", help="Default: every fetched set."),
    limit: int = typer.Option(0, help="Clips per set (0: all)."),
    redo: bool = False,
) -> None:
    """Speech models on the public sets."""
    asr_bench.run(models, set_names, limit, redo)
    asr_bench.report()


@bench_app.command("report")
def bench_report() -> None:
    """Word error rates of every model on every public set."""
    asr_bench.report()


@bench_app.command("compare")
def bench_compare(a: str, b: str, lang: str = "en", resamples: int = 10_000) -> None:
    """Paired bootstrap of two models' word error rates."""
    asr_bench.compare(a, b, lang, resamples)
