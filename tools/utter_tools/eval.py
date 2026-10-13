"""utter-eval: how often each cleanup model types what the speaker meant, ready to send.

The sets are the datasets' test splits at one revision: the text test split (the transcripts the
dataset records) and every speech eval collection (recognized here by Utter's own recognizer);
the two sets with a user's dictionary are also run with it, boosted in the recognizer and sent to
cleanup. Every model runs through utter-cli, the app's own client, prompt and server settings.

A dictation is ready to send if the output is the final text (spacing aside) or if the judge
(Gemma 4 26B, prompts/judge.md, calibrated on the maintainer's review) finds only equally valid choices
differ: it reads the outputs, it writes none. Every step resumes: what is on disk is not redone.

    utter-eval --model quality-v2=PATH.gguf --model quality-v1=PATH.gguf [--judge-url URL]
"""

from __future__ import annotations

import hashlib
import io
import itertools
import json
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import soundfile as sf
import typer

from utter_tools import common, llm, schema
from utter_tools.text import relevant_terms

TEXT_SET = "text-test"
ASR = "parakeet"
DICTIONARIES = Path(__file__).parent / "dictionaries"
# Sets also run with a user's dictionary (sampled from the set's own terms).
DICTIONARY_SETS = {
    "kokoro-real": DICTIONARIES / "kokoro-real-300.txt",
    "kokoro-directed": DICTIONARIES / "kokoro-directed-231.txt",
}
FINE_TUNED_PROMPT = common.REPO / "crates" / "utter-llm" / "prompts" / "cleanup-ft.md"
# The app's cleanup server settings (crates/utter-llm/src/cleanup.rs).
SERVER_ARGS = [
    "--swa-full",
    "-ub",
    "128",
    "--spec-type",
    "ngram-simple",
    "--spec-ngram-simple-size-n",
    "3",
    "--spec-ngram-simple-size-m",
    "6",
]
CAUSES = [
    "style",
    "recognition",
    "punctuation",
    "capitalization",
    "grammar",
    "correction",
    "list",
    "numbers-symbols",
    "words",
    "other",
]
VERDICT = {
    "type": "object",
    "properties": {
        "differences": {"type": "array", "items": {"type": "string"}},
        "verdict": {"enum": ["acceptable", "needs_edit"]},
        "cause": {"enum": CAUSES},
        "note": {"type": "string"},
    },
    "required": ["differences", "verdict", "cause", "note"],
}


def results(kind: str, run: str, eval_set: str) -> Path:
    return common.eval_dir() / "results" / kind / run / f"{eval_set}.jsonl"


def manifest(eval_set: str) -> list[dict]:
    return common.read_jsonl(common.eval_dir() / eval_set / "manifest.jsonl")


# ---------------------------------------------------------------------------------------------
# The sets.


def _made_from(folder: Path, source: dict) -> bool:
    marker = folder / "source.json"
    return marker.is_file() and json.loads(marker.read_text()) == source


def _set_source(eval_set: str) -> dict:
    """What an eval set was made from (make_sets): the dataset, its revision, the limit."""
    return json.loads((common.eval_dir() / eval_set / "source.json").read_text())


def _stamp(results_path: Path) -> Path:
    return results_path.with_suffix(".source.json")


def _fresh(results_path: Path, source: dict) -> bool:
    """Whether a results file was made from `source`: a new revision of the set, another model
    file or recognizer run makes it stale, and it is made again."""
    stamp = _stamp(results_path)
    return stamp.is_file() and json.loads(stamp.read_text()) == source


def _file_id(path: Path) -> dict:
    """Which file `path` is (a rebuilt model under the same name is another): without hashing
    gigabytes."""
    st = path.stat()
    return {"path": str(path.resolve()), "bytes": st.st_size, "modified": int(st.st_mtime)}


def make_sets(text_ref: str | None, speech_ref: str | None, limit: int = 0) -> list[str]:
    """The eval sets from the datasets at those revisions (default: main), as `utter-cli bench`
    reads them: a manifest of dataset rows and, for speech, a WAV per clip; with `limit`, the
    first rows of each. Returns their names."""
    text_rev = common.revision(common.TEXT_REPO, text_ref)
    speech_rev = common.revision(common.SPEECH_REPO, speech_ref)
    folder = common.eval_dir() / TEXT_SET
    source = {"repo": common.TEXT_REPO, "revision": text_rev, "limit": limit}
    if not _made_from(folder, source):
        columns = [c for c in schema.TEXT.names if c != "messages"]
        rows = common.text_rows(text_rev, "test", columns)
        rows = rows[:limit] if limit else rows
        common.write_jsonl(folder / "manifest.jsonl", rows)
        common.write_jsonl(
            results("asr", "dictated", TEXT_SET),
            [{"id": r["id"], "hyp": r["dictated"]} for r in rows],
        )
        (folder / "source.json").write_text(json.dumps(source))
    names = [TEXT_SET]
    # The corpus's test clips are the text test split's rows, which text-test covers.
    for collection in common.speech_collections(speech_rev):
        if collection.startswith(schema.CORPUS):
            continue
        names.append(collection)
        folder = common.eval_dir() / collection
        source = {"repo": common.SPEECH_REPO, "revision": speech_rev, "limit": limit}
        if _made_from(folder, source):
            continue
        folder.mkdir(parents=True, exist_ok=True)
        rows = []
        clips = common.speech_rows(speech_rev, collection, "test")
        for r in itertools.islice(clips, limit or None):
            audio, rate = sf.read(io.BytesIO(r["audio"]["bytes"]), dtype="int16")
            wav = f"{r['rendition_id']}.wav"
            sf.write(folder / wav, audio, rate, subtype="PCM_16")
            rows.append({**r, "audio": wav})
        common.write_jsonl(folder / "manifest.jsonl", rows)
        (folder / "source.json").write_text(json.dumps(source))
    return names


# ---------------------------------------------------------------------------------------------
# Recognition and cleanup, through utter-cli.


def _done(path: Path, eval_set: str, field: str) -> bool:
    have = {r["id"] for r in common.read_jsonl(path) if field in r}
    return have >= {r["id"] for r in manifest(eval_set)}


def recognize(eval_set: str, run: str, dictionary: Path | None = None) -> None:
    out = results("asr", run, eval_set)
    source = {"set": _set_source(eval_set), "dictionary": str(dictionary) if dictionary else None}
    if _fresh(out, source) and _done(out, eval_set, "hyp"):
        return
    # utter-cli bench resumes from what is there: stale results must not be.
    out.unlink(missing_ok=True)
    cmd = [common.utter_cli(), "bench", "--sets", eval_set, "--name", run]
    if dictionary:
        cmd += ["--phrases", str(dictionary)]
    subprocess.run(cmd, check=True)
    _stamp(out).write_text(json.dumps(source))


def clean(eval_set: str, run: str, gguf: Path, asr_run: str, dictionary: Path | None) -> None:
    """The model's output for every dictation, with what the app sends with it: the dictionary
    terms heard (a text row's own, or those of `dictionary`) and the lists setting."""
    out = results("llm", run, eval_set)
    source = {
        "set": _set_source(eval_set),
        "model": _file_id(gguf),
        "asr": asr_run,
        "dictionary": str(dictionary) if dictionary else None,
    }
    if _fresh(out, source) and _done(out, eval_set, "output"):
        return
    rows = {r["id"]: r for r in manifest(eval_set)}
    lines = dictionary.read_text(encoding="utf-8").splitlines() if dictionary else []
    terms = [t.strip() for t in lines if t.strip()]
    cases = []
    for h in common.read_jsonl(results("asr", asr_run, eval_set)):
        row = rows[h["id"]]
        case = {"id": h["id"], "input": h["hyp"]}
        heard = row.get("dictionary") or relevant_terms(h["hyp"], terms)
        if heard:
            case["dictionary"] = heard
        if row.get("lists") is False:
            case["lists_off"] = True
        cases.append(case)
    # llm-bench writes its outputs over its cases file.
    common.write_jsonl(out, cases)
    cmd = [common.utter_cli(), "llm-bench", "--gguf", str(gguf), "--name", run, "--cases", str(out)]
    cmd += ["--prompt", str(FINE_TUNED_PROMPT)] + [f"--server-arg={a}" for a in SERVER_ARGS]
    subprocess.run(cmd, check=True)
    _stamp(out).write_text(json.dumps(source))


# ---------------------------------------------------------------------------------------------
# Ready to send.


def _same(a: str, b: str) -> bool:
    return " ".join(a.split()) == " ".join(b.split())


def ready_to_send(eval_set: str, run: str, chat: llm.Chat, judge: str, workers: int) -> dict:
    """Judges the outputs not judged yet; returns ready-to-send share, count and causes."""
    gold = {r["id"]: r["final_text"] for r in manifest(eval_set)}
    outputs = common.read_jsonl(results("llm", run, eval_set))
    path = results("judge", f"{run}--{judge}", eval_set)
    system = common.prompt("judge")
    rubric = hashlib.sha256(system.encode()).hexdigest()[:12]
    # A verdict holds what it judged: the output, the intended text and the rubric. One whose
    # output, gold (a new dataset revision) or rubric has changed since is not reused.
    judged = {o["id"]: (o["output"], gold.get(o["id"]), rubric) for o in outputs}
    verdicts = {
        v["id"]: v
        for v in common.read_jsonl(path)
        if (v.get("output"), v.get("meant"), v.get("rubric")) == judged.get(v["id"])
    }
    todo = [o for o in outputs if o["id"] not in verdicts and not _same(o["output"], gold[o["id"]])]

    def judge_one(o: dict) -> dict:
        user = f"Heard: {o['input']}\n\nMeant: {gold[o['id']]}\n\nTyped: {o['output']}"
        reply = chat.ask(system, user, VERDICT, seed=0, max_tokens=600)
        return {
            "id": o["id"],
            "output": o["output"],
            "meant": gold[o["id"]],
            "rubric": rubric,
            **(reply or {"verdict": "needs_edit", "cause": "other", "note": "unreadable"}),
        }

    with ThreadPoolExecutor(workers) as pool:
        for v in pool.map(judge_one, todo):
            common.write_jsonl(path, [v], append=True)
            verdicts[v["id"]] = v
    ready = sum(
        _same(o["output"], gold[o["id"]]) or verdicts[o["id"]]["verdict"] == "acceptable"
        for o in outputs
    )
    causes: dict[str, int] = {}
    for v in verdicts.values():
        if v["verdict"] == "needs_edit":
            causes[v["cause"]] = causes.get(v["cause"], 0) + 1
    return {"ready_pct": 100 * ready / max(1, len(outputs)), "n": len(outputs), "causes": causes}


# ---------------------------------------------------------------------------------------------
# The command.


def _models(specs: list[str]) -> dict[str, Path]:
    models = {}
    for spec in specs:
        name, _, path = spec.partition("=")
        if not path or not Path(path).is_file():
            raise typer.BadParameter(f"{spec}: expected NAME=PATH to a .gguf file")
        models[name] = Path(path)
    return models


def main(
    model: list[str] = typer.Option(..., help="NAME=PATH.gguf, repeatable: the models to compare."),
    judge_url: str = typer.Option(
        "", help="The judge: an OpenAI-style server (vLLM); else the local 26B."
    ),
    judge: str = typer.Option(
        "", help="The judge's name in results (default: vllm-26b or local-26b)."
    ),
    workers: int = typer.Option(8, help="Judge requests at once (64 suits vLLM)."),
    text_ref: str = typer.Option(None, help="dictation-to-text revision (default: main)."),
    speech_ref: str = typer.Option(None, help="dictation-speech revision (default: main)."),
    limit: int = typer.Option(0, help="Only the first rows of each set, for a quick run (0: all)."),
) -> None:
    """How often each cleanup model types what the speaker meant, on the datasets' test splits."""
    models = _models(model)
    judge = judge or ("vllm-26b" if judge_url else "local-26b")
    sets = make_sets(text_ref, speech_ref, limit)
    # (set, recognizer run, run suffix, dictionary) for every column of the table.
    runs: list[tuple[str, str, str, Path | None]] = [(TEXT_SET, "dictated", "", None)]
    for s in sets[1:]:
        recognize(s, ASR)
        runs.append((s, ASR, "", None))
        if s in DICTIONARY_SETS:
            recognize(s, f"{ASR}-dict", DICTIONARY_SETS[s])
            runs.append((s, f"{ASR}-dict", "-dict", DICTIONARY_SETS[s]))
    for name, gguf in models.items():
        for s, asr_run, suffix, dictionary in runs:
            clean(s, name + suffix, gguf, asr_run, dictionary)

    lines = ["| Set | n | " + " | ".join(models) + " |", "|---|---|" + "---|" * len(models)]
    with llm.connect(judge_url, slots=workers) as chat:
        for s, _, suffix, dictionary in runs:
            cells, n = [], 0
            for name in models:
                result = ready_to_send(s, name + suffix, chat, judge, workers)
                cells.append(f"{result['ready_pct']:.1f}")
                n = result["n"]
            label = f"{s} + dictionary" if dictionary else s
            lines.append(f"| {label} | {n} | " + " | ".join(cells) + " |")
    table = "Ready to send %:\n\n" + "\n".join(lines) + "\n"
    out = common.eval_dir() / "results" / f"eval--{'--'.join(models)}--{judge}.md"
    out.write_text(table, encoding="utf-8", newline="\n")
    print(table)
    print(f"written to {out}")


def cli() -> None:
    typer.run(main)
