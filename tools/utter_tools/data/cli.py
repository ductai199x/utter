"""utter-data: grow the published datasets, one batch at a time (data/batch.py).

utter-data new NAME --collection C [--seed S] [--test-only]
utter-data passages NAME --source sources|lists|everyday-lists|messages|directed
utter-data fix NAME            proofread and grammar-fix the passages (before rendering)
utter-data render NAME         spoken versions, in each kind
utter-data speak NAME          speech (Kokoro or Chatterbox)
utter-data transcribe NAME     Utter's recognizer on the speech
utter-data pairs NAME          text rows (not for an eval batch: eval sets are speech only)
utter-data build NAME          the datasets with the batch merged in, and their cards
utter-data audit NAME          every check the datasets promise
utter-data upload NAME --yes   publish (only after a clean audit)
utter-data status NAME
"""

from __future__ import annotations

import collections

import typer

from utter_tools import config, hub, llm
from utter_tools.data import (
    audit,
    goldfix,
    grammar,
    lists,
    messages,
    pairs,
    passages,
    publish,
    speech,
    spoken,
)
from utter_tools.data.batch import Batch, append_jsonl, published_passages, read_jsonl, write_jsonl

app = typer.Typer(
    add_completion=False,
    no_args_is_help=True,
    help="Grow the published datasets, one batch at a time.",
)

LLM_URL = typer.Option("", help="An OpenAI-style server (vLLM on FLAME); else the local 26B.")
WORKERS = typer.Option(8, help="Requests at once (64 suits vLLM).")
SOURCES = ("sources", "lists", "everyday-lists", "messages", "directed")


def _fail(message: str) -> None:
    typer.echo(message, err=True)
    raise typer.Exit(1)


@app.command()
def new(
    name: str,
    collection: str = typer.Option(..., help="The speech collection its clips are published as."),
    seed: int = typer.Option(
        0, help="Seeds everything random in the batch; give each batch its own."
    ),
    test_only: bool = typer.Option(False, help="An eval set: speech only, every clip in test."),
) -> None:
    """Start a batch."""
    if collection in hub.speech_collections(config.SPEECH_REPO, hub.revision(config.SPEECH_REPO)):
        _fail(f"collection {collection} is already published")
    b = Batch.create(name, seed, test_only, collection)
    typer.echo(f"batch {name} at {b.root} (commit {b.meta.commit[:10]})")


@app.command("passages")
def passages_cmd(
    name: str,
    source: str = typer.Option(..., help=" | ".join(SOURCES)),
    count: int = typer.Option(200, help="Per domain (sources), per shard (lists), or in all."),
    llm_url: str = LLM_URL,
    workers: int = WORKERS,
) -> None:
    """Add passages to a batch (never one already published)."""
    b = Batch(name)
    if b.renditions_path.exists():
        _fail("the batch is rendered already: passages are added before rendering")
    seed = b.meta.seed
    if source == "sources":
        rows = passages.collect(count, seed)
    elif source == "lists":
        rows = lists.extract(count, seed)
    elif source in ("everyday-lists", "messages", "directed"):
        with llm.connect(llm_url) as chat:
            if source == "everyday-lists":
                rows = lists.synth(count, seed, chat, b.meta.commit, workers)
            else:
                rows = messages.generate(
                    count, seed, chat, b.meta.commit, source == "directed", workers
                )
    else:
        raise typer.BadParameter(f"{source}: one of {', '.join(SOURCES)}")
    added = b.add_passages(rows, published_passages())
    typer.echo(f"{added} of {len(rows)} passages added ({len(b.passages())} in the batch)")


@app.command()
def fix(name: str, llm_url: str = LLM_URL, workers: int = WORKERS) -> None:
    """Proofread the passages, then fix their grammar; drops what is not dictatable prose."""
    b = Batch(name)
    if b.renditions_path.exists():
        _fail("the batch is rendered already: its spoken versions were made from these passages")
    # Each run starts from the passages as they were added, so a rerun redoes, never stacks.
    original = b.root / "passages.unfixed.jsonl"
    if not original.exists():
        b.passages_path.replace(original)
    rows = read_jsonl(original)
    with llm.connect(llm_url, slots=workers) as chat:
        for grammar_pass in (False, True):
            rows, dropped = goldfix.fix(rows, chat, grammar_pass, workers)
            typer.echo(
                f"{'grammar' if grammar_pass else 'proofreading'}: {len(rows)} kept, dropped {dropped}"
            )
    write_jsonl(b.passages_path, rows)


@app.command()
def render(
    name: str,
    kind: list[str] = typer.Option(list(spoken.KINDS), help="Kinds to render, repeatable."),
    per_passage: int = typer.Option(0, help="Kinds per passage, chosen by its id (0: every kind)."),
    grammar_boost: str = typer.Option(
        "", help='Grammar error types weighted up, e.g. "DET=2,PREP=2".'
    ),
    llm_url: str = LLM_URL,
    workers: int = WORKERS,
) -> None:
    """Spoken versions of every passage; resumes where a run stopped."""
    b = Batch(name)
    unknown = set(kind) - set(spoken.KINDS) - set(spoken.EXTRA_KINDS)
    if unknown:
        raise typer.BadParameter(f"unknown kinds {sorted(unknown)}")
    if grammar_boost:
        grammar.BOOST.update(
            {t: float(w) for t, w in (p.split("=") for p in grammar_boost.split(","))}
        )
    done = {r["id"] for r in read_jsonl(b.renditions_path)}
    kept = failed = 0
    with llm.connect(llm_url, slots=workers) as chat:
        for rid, row, rejections in spoken.render(
            chat, b.passages(), kind, done, b.meta.seed, per_passage, workers
        ):
            if row is None:
                failed += 1
                typer.echo(f"{rid}: no rendition passed ({rejections})")
                continue
            append_jsonl(b.renditions_path, row)
            kept += 1
    typer.echo(f"{kept} renditions added, {failed} failed ({len(done) + kept} in the batch)")


@app.command()
def speak(
    name: str,
    engine: str = typer.Option("kokoro", help="kokoro | chatterbox"),
    voices: str = typer.Option(
        "", help="Chatterbox's voice share: eval | train (default: by the batch)."
    ),
    gpus: str = typer.Option("0", help="CUDA devices, comma-separated."),
    per_gpu: int = typer.Option(1, help="Speaking processes per GPU."),
) -> None:
    """Speak the batch's renditions; resumes where a run stopped."""
    b = Batch(name)
    share = voices or ("eval" if b.meta.test_only else "train")
    clips = speech.speak(b, engine, share, gpus.split(","), per_gpu)
    typer.echo(f"{clips} clips in {b.clips}")


@app.command()
def transcribe(name: str) -> None:
    """Utter's recognizer on the batch's clips."""
    heard = speech.transcribe(Batch(name))
    typer.echo(f"{len(heard)} clips transcribed")


@app.command("pairs")
def pairs_cmd(
    name: str,
    lists_off_share: float = typer.Option(0.2, help="Share of list renditions with lists off."),
    dictionary_share: float = typer.Option(0.5, help="Share of rows whose user has a dictionary."),
) -> None:
    """Text rows from the transcribed clips."""
    b = Batch(name)
    if b.meta.test_only:
        _fail("an eval batch is speech only: no text rows")
    clips = read_jsonl(b.clips / "manifest.jsonl")
    heard = speech.transcribe(b)
    # The dictionary pool: every term the batch's and the published training rows could hold.
    rev = hub.revision(config.TEXT_REPO)
    golds = [c["final_text"] for c in clips] + [
        r["final_text"] for r in hub.text_rows(config.TEXT_REPO, rev, "train", ["final_text"])
    ]
    rows = pairs.build(
        clips, heard, pairs.TermPool(golds), b.meta.seed, lists_off_share, dictionary_share
    )
    write_jsonl(b.text_path, rows)
    typer.echo(f"{len(rows)} text rows from {len(clips)} clips -> {b.text_path}")


@app.command()
def build(name: str) -> None:
    """The datasets with the batch merged in."""
    info = publish.build(Batch(name))
    for k, v in info.items():
        typer.echo(f"{k}: {v}")


def _audit(b: Batch) -> list[str]:
    out = publish.folder(b)
    info = publish.build_info(b)
    failures: list[str] = []
    if not b.meta.test_only:
        typer.echo("dictation-to-text")
        failures += audit.text(out / "text")
        text = audit.text_rows(out / "text")
    else:
        rev = info[config.TEXT_REPO]
        text = [
            {**r, "_split": s}
            for s in ("train", "test")
            for r in hub.text_rows(config.TEXT_REPO, rev, s, ["id", "final_text", "lists"])
        ]
    typer.echo("dictation-speech")
    failures += audit.speech(out / "speech", text, b.clips, info[config.SPEECH_REPO])
    return failures


@app.command("audit")
def audit_cmd(name: str) -> None:
    """Every check the datasets promise, on the batch's build."""
    failures = _audit(Batch(name))
    if failures:
        _fail(f"{len(failures)} checks failed")
    typer.echo("every check passed")


@app.command()
def upload(
    name: str, yes: bool = typer.Option(False, "--yes", help="Publish (public datasets).")
) -> None:
    """Publish the batch's build, after a clean audit."""
    b = Batch(name)
    if _audit(b):
        _fail("the audit failed: nothing uploaded")
    if not yes:
        _fail("the audit passed; add --yes to publish")
    publish.upload(b)
    typer.echo("published")


@app.command()
def status(name: str) -> None:
    """Where a batch is."""
    b = Batch(name)
    renditions = read_jsonl(b.renditions_path)
    clips = read_jsonl(b.clips / "manifest.jsonl")
    typer.echo(
        f"batch {name}: collection {b.meta.collection}, seed {b.meta.seed}, test-only {b.meta.test_only}"
    )
    typer.echo(f"  passages {len(b.passages())}")
    typer.echo(
        f"  renditions {len(renditions)} {dict(collections.Counter(r['category'] for r in renditions))}"
    )
    typer.echo(f"  clips {len(clips)}")
    typer.echo(f"  text rows {len(read_jsonl(b.text_path))}")
    typer.echo(f"  built {(publish.folder(b) / 'build.json').exists()}")
