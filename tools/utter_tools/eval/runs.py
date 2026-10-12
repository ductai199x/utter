"""Runs over the eval sets, through Utter's own Rust runtime (utter-cli), so the eval sees exactly
what the app does:

- `asr`: Parakeet on a speech set (`utter-cli bench`), optionally boosting a dictionary's
  phrases, into `results/asr/<run>/<set>.jsonl`;
- `clean`: a cleanup model on a recognizer run's transcripts (`utter-cli llm-bench`: the app's
  client, prompt and server settings), into `results/llm/<run>/<set>-input.jsonl`. Each request
  carries what the app's would: the dictionary terms heard in it (dictionary.relevant_terms, the
  mirror of gate::relevant_terms) and the lists setting.

Both resume: a set whose output is complete is skipped.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from utter_tools import config
from utter_tools.eval import dictionary
from utter_tools.eval.sets import asr_path, llm_path, manifest, read_jsonl
from utter_tools.text.dictionary import relevant_terms

# The fine-tuned models' prompt (Tier::system_prompt in crates/utter-llm/src/cleanup.rs).
FINE_TUNED_PROMPT = config.REPO / "crates" / "utter-llm" / "prompts" / "cleanup-ft.md"
# The app's cleanup server settings (crates/utter-llm/src/cleanup.rs, SERVER_ARGS).
SERVER_ARGS = [
    "--swa-full",
    "-ub", "128",
    "--spec-type", "ngram-simple",
    "--spec-ngram-simple-size-n", "3",
    "--spec-ngram-simple-size-m", "6",
]  # fmt: skip


def _complete(path: Path, ids: set[str], field: str) -> bool:
    if not path.is_file():
        return False
    rows = read_jsonl(path)
    return {r["id"] for r in rows if field in r} >= ids


def asr(eval_set: str, run: str, phrases: Path | None = None) -> None:
    ids = {r["id"] for r in manifest(eval_set)}
    if _complete(asr_path(run, eval_set), ids, "hyp"):
        return
    cmd = [str(config.utter_cli()), "bench", "--sets", eval_set, "--name", run]
    if phrases is not None:
        cmd += ["--phrases", str(phrases)]
    subprocess.run(cmd, check=True)


def clean(
    eval_set: str,
    run: str,
    gguf: Path,
    asr_run: str,
    phrases: Path | None = None,
    prompt: Path = FINE_TUNED_PROMPT,
) -> None:
    rows = {r["id"]: r for r in manifest(eval_set)}
    out = llm_path(run, eval_set)
    if _complete(out, set(rows), "output"):
        return
    terms = dictionary.read(phrases) if phrases else []
    cases = []
    for h in read_jsonl(asr_path(asr_run, eval_set)):
        row = rows[h["id"]]
        case: dict = {"id": h["id"], "input": h["hyp"]}
        # The text test split's rows carry their own dictionary and lists setting.
        heard = row.get("dictionary") or relevant_terms(h["hyp"], terms)
        if heard:
            case["dictionary"] = heard
        if row.get("lists") is False:
            case["lists_off"] = True
        cases.append(case)
    out.parent.mkdir(parents=True, exist_ok=True)
    # llm-bench writes its results over the cases file, named after it.
    with out.open("w", encoding="utf-8", newline="\n") as fh:
        for c in cases:
            fh.write(json.dumps(c, ensure_ascii=False) + "\n")
    cmd = [str(config.utter_cli()), "llm-bench", "--gguf", str(gguf), "--name", run]
    cmd += ["--cases", str(out), "--prompt", str(prompt)]
    cmd += [f"--server-arg={a}" for a in SERVER_ARGS]
    subprocess.run(cmd, check=True)
