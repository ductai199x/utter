"""utter-data: grow the published datasets, one batch at a time (batch.py), each step resuming:

utter-data new NAME --collection C --seed S [--test-only]
utter-data passages NAME --source sources|lists|everyday-lists|messages|directed
utter-data fix NAME            proofread the passages, then fix their grammar
utter-data render NAME         spoken versions in each category
utter-data speak NAME          the speech (Kokoro, or Chatterbox's voice clones)
utter-data pairs NAME          Parakeet on the speech, then text rows (not for an eval set)
utter-data build NAME          the datasets with the batch merged in, and their cards
utter-data check NAME          everything the datasets promise
utter-data upload NAME --yes   publish, after a clean check
"""

from __future__ import annotations

import collections
import os
import subprocess

import typer

from utter_tools import common, llm
from utter_tools.data import goldfix, grammar, lists, messages, pairs, passages, publish, spoken
from utter_tools.data.batch import Batch, ids

app = typer.Typer(
    add_completion=False,
    no_args_is_help=True,
    help="Grow the published datasets, one batch at a time.",
)
LLM_URL = typer.Option("", help="An OpenAI-style server (vLLM on a GPU cluster); else the local 26B.")
WORKERS = typer.Option(8, help="LLM requests at once (64 suits vLLM).")
KINDS = typer.Option(
    list(spoken.KINDS),
    help=f"Categories to speak in, repeatable; also {', '.join(spoken.EXTRA_KINDS)}.",
)
SPEAK = common.REPO / "tools" / "envs" / "tts" / "speak.py"


def _stop(message: str) -> None:
    typer.echo(message, err=True)
    raise typer.Exit(1)


@app.command()
def new(
    name: str,
    collection: str = typer.Option(..., help="The speech collection its clips are published as."),
    seed: int = typer.Option(..., help="Seeds everything random in the batch: give each its own."),
    test_only: bool = typer.Option(False, help="An eval set: speech only, every clip in test."),
) -> None:
    """Start a batch."""
    if collection in common.speech_collections(common.revision(common.SPEECH_REPO)):
        _stop(f"collection {collection} is published already")
    batch = Batch.create(name, seed, collection, test_only)
    typer.echo(f"{batch.root} (commit {batch.meta.commit[:10]})")


@app.command("passages")
def add_passages(
    name: str,
    source: str = typer.Option(..., help="sources | lists | everyday-lists | messages | directed"),
    count: int = typer.Option(200, help="Per domain (sources), per shard (lists), else in all."),
    llm_url: str = LLM_URL,
    workers: int = WORKERS,
) -> None:
    """Add passages (never one already published)."""
    batch = Batch(name)
    if batch.renditions.exists():
        _stop("the batch is rendered already: add passages before rendering")
    seed = batch.meta.seed
    if source == "sources":
        rows = passages.collect(count, seed)
    elif source == "lists":
        rows = lists.extract(count, seed)
    elif source == "everyday-lists":
        with llm.connect(llm_url, workers) as chat:
            rows = lists.synth(count, seed, chat, batch.code_url("lists"), workers)
    elif source in ("messages", "directed"):
        with llm.connect(llm_url, workers) as chat:
            url = batch.code_url("messages")
            rows = messages.generate(count, seed, chat, url, source == "directed", workers)
    else:
        raise typer.BadParameter(f"unknown source {source}")
    typer.echo(f"{batch.add_passages(rows)} of {len(rows)} passages added")


@app.command()
def fix(name: str, llm_url: str = LLM_URL, workers: int = WORKERS) -> None:
    """Proofread the passages, then fix their grammar (before they are spoken)."""
    batch = Batch(name)
    if batch.renditions.exists():
        _stop("the batch is rendered already: its spoken versions come from these passages")
    # Each run starts from the passages as added, so a rerun redoes the fixes, never stacks them.
    original = batch.root / "passages.unfixed.jsonl"
    if not original.exists():
        batch.passages.replace(original)
    rows = common.read_jsonl(original)
    with llm.connect(llm_url, workers) as chat:
        for grammar_pass in (False, True):
            rows, dropped = goldfix.fix(rows, chat, grammar_pass, workers)
            step = "grammar" if grammar_pass else "proofreading"
            typer.echo(f"{step}: {len(rows)} kept, dropped {dropped}")
    common.write_jsonl(batch.passages, rows)


@app.command()
def render(
    name: str,
    kind: list[str] = KINDS,
    per_passage: int = typer.Option(0, help="Categories per passage, chosen by its id (0: all)."),
    grammar_boost: str = typer.Option(
        "", help='Grammar error types weighted up, e.g. "DET=2,PREP=2".'
    ),
    llm_url: str = LLM_URL,
    workers: int = WORKERS,
) -> None:
    """Spoken versions of every passage."""
    batch = Batch(name)
    unknown = set(kind) - set(spoken.KINDS + spoken.EXTRA_KINDS)
    if unknown:
        raise typer.BadParameter(f"unknown categories {sorted(unknown)}")
    for pair in filter(None, grammar_boost.split(",")):
        error_type, weight = pair.split("=")
        grammar.BOOST[error_type] = float(weight)
    done = ids(batch.renditions)
    kept = failed = 0
    with llm.connect(llm_url, workers) as chat:
        jobs = spoken.render(
            chat,
            common.read_jsonl(batch.passages),
            kind,
            done,
            batch.meta.seed,
            per_passage,
            workers,
        )
        for rendition_id, row, rejections in jobs:
            if row is None:
                failed += 1
                typer.echo(f"{rendition_id}: no version passed the checks ({rejections})")
            else:
                common.write_jsonl(batch.renditions, [row], append=True)
                kept += 1
    typer.echo(f"{kept} renditions added, {failed} failed")


@app.command()
def speak(
    name: str,
    engine: str = typer.Option("kokoro", help="kokoro | chatterbox"),
    gpus: str = typer.Option("0", help="CUDA devices, comma-separated."),
    per_gpu: int = typer.Option(1, help="Speaking processes per GPU."),
) -> None:
    """Speak the renditions (in the engine's own environment, envs/tts-<engine>)."""
    batch = Batch(name)
    # An eval set speaks with voices no training clip uses (Chatterbox's eval share).
    voices = "eval" if batch.meta.test_only else "train"
    devices = gpus.split(",") * per_gpu
    procs = []
    for i, device in enumerate(devices):
        cmd = [
            "uv",
            "run",
            "--project",
            str(SPEAK.parents[1] / f"tts-{engine}"),
            "python",
            str(SPEAK),
        ]
        cmd += [
            engine,
            str(batch.renditions),
            str(batch.clips),
            "--collection",
            batch.meta.collection,
        ]
        cmd += ["--voices", voices, "--shard", f"{i}/{len(devices)}"]
        procs.append(subprocess.Popen(cmd, env={**os.environ, "CUDA_VISIBLE_DEVICES": device}))
    if sum(p.wait() != 0 for p in procs):
        _stop("a speaking process failed")
    # One manifest of the clips whose audio exists, the latest row per clip.
    clips = {}
    for part in sorted(batch.clips.glob("manifest*.jsonl")):
        clips |= {
            c["id"]: c for c in common.read_jsonl(part) if (batch.clips / c["audio"]).is_file()
        }
    common.write_jsonl(
        batch.clips / "manifest.jsonl", sorted(clips.values(), key=lambda c: c["id"])
    )
    for part in batch.clips.glob("manifest.*-of-*.jsonl"):
        part.unlink()
    typer.echo(f"{len(clips)} clips in {batch.clips}")


@app.command("pairs")
def make_pairs(
    name: str,
    lists_off_share: float = typer.Option(0.2, help="Share of list renditions with lists off."),
    dictionary_share: float = typer.Option(0.5, help="Share of rows whose user has a dictionary."),
) -> None:
    """Parakeet on the speech (utter-cli bench), then text rows."""
    batch = Batch(name)
    if batch.meta.test_only:
        _stop("an eval set is speech only: no text rows")
    run = common.eval_dir() / "results" / "asr" / "parakeet" / f"{batch.meta.collection}.jsonl"
    if not run.exists():
        subprocess.run(
            [common.utter_cli(), "bench", "--sets", batch.meta.collection, "--name", "parakeet"],
            check=True,
        )
    heard = {r["id"]: r["hyp"] for r in common.read_jsonl(run)}
    clips = common.read_jsonl(batch.clips / "manifest.jsonl")
    # Users' dictionaries are drawn from the terms of the batch and of the published training rows.
    published = common.text_rows(common.revision(common.TEXT_REPO), "train", ["final_text"])
    pool = pairs.TermPool([c["final_text"] for c in clips] + [r["final_text"] for r in published])
    rows = pairs.build(clips, heard, pool, batch.meta.seed, lists_off_share, dictionary_share)
    common.write_jsonl(batch.text, rows)
    typer.echo(f"{len(rows)} text rows from {len(clips)} clips")


@app.command()
def build(name: str) -> None:
    """The datasets with the batch merged in."""
    typer.echo(publish.build(Batch(name)))


@app.command()
def check(name: str) -> None:
    """Everything the datasets promise, on the build."""
    failed = publish.check(Batch(name))
    if failed:
        _stop(f"{len(failed)} checks failed")
    typer.echo("every check passed")


@app.command()
def upload(
    name: str, yes: bool = typer.Option(False, "--yes", help="Publish: the datasets are public.")
) -> None:
    """Publish the build after a clean check."""
    batch = Batch(name)
    if publish.check(batch):
        _stop("the checks failed: nothing uploaded")
    if not yes:
        _stop("every check passed; add --yes to publish")
    publish.upload(batch)
    typer.echo("published")


@app.command()
def status(name: str) -> None:
    """Where a batch is."""
    batch = Batch(name)
    meta = batch.meta
    renditions = common.read_jsonl(batch.renditions)
    categories = dict(collections.Counter(r["category"] for r in renditions))
    typer.echo(
        f"{name}: collection {meta.collection}, seed {meta.seed}, test-only {meta.test_only}"
    )
    typer.echo(f"  passages    {len(common.read_jsonl(batch.passages))}")
    typer.echo(f"  renditions  {len(renditions)} {categories}")
    typer.echo(f"  clips       {len(common.read_jsonl(batch.clips / 'manifest.jsonl'))}")
    typer.echo(f"  text rows   {len(common.read_jsonl(batch.text))}")
    typer.echo(f"  built       {(publish.folder(batch) / 'build.json').exists()}")


def cli() -> None:
    app()
