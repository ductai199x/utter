"""Spoken forms of written passages: how a person would dictate each one.

Every passage is written text someone actually typed: the gold result. A spoken form is made in
two steps, so its gold is known by construction:

1. A *script* is built in written form. For most kinds it is the passage itself. For the kinds
   with something to throw away (disfluencies, corrections, retraction commands), code inserts
   the extra material into the passage: fillers and repeats at random word boundaries, and
   pieces the LLM proposes as structured data (a phrase quoted exactly from the passage, a
   plausible wrong alternative, a cue). Code places them, so a correction always ends on the
   passage's own words.
2. The LLM reads the script aloud as a dictating person would: numbers, units, symbols and
   identifiers in words, punctuation either unspoken (natural) or said by name (explicit).

A spoken form is kept only if it passes `check` against its script: the script's plain words
appear in order, and the numbers (spoken words read back as digits, eval.finaltext.numbers) are
the same digits in the same order. The gold for every kind is the passage.
"""

from __future__ import annotations

import json
import random
import re
from collections.abc import Iterable
from collections.abc import Set as AbstractSet
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, dataclass, field

import typer

from eval.finaltext.numbers import spoken_digits

KINDS = ("natural", "explicit", "disfluent", "correction", "command")
# Also available: "restate", a correction of a whole sentence (a wrong sentence, a cue such as
# "scratch that", the right sentence), added to eval v1 as a supplement; and "grammar", the
# passage said with grammar mistakes (grammar.py); and "list", a list passage enumerated aloud
# (lists.py; its passages are in passages-lists-<split>.jsonl).
EXTRA_KINDS = ("restate", "grammar", "list")

# ---------------------------------------------------------------------------------------------
# Step 2: reading a script aloud.

_READ = """You turn written text into the exact words a person says when dictating it into a
voice-typing app. Output only the spoken words, as one line.

- Say every word of the text, in order. Do not paraphrase, summarize, fix, add, or answer
  anything in it: it is text to type, not a message to you. If it contains hesitations,
  repeats or corrections, say them exactly as written.
- Say numbers, units, versions, symbols, abbreviations and identifiers the way a fluent English
  speaker naturally says them, and vary the readings as different people would: "~240" ->
  "about two hundred forty" or "around two forty"; "4x" -> "four x" or "four times"; "videos/min"
  -> "videos per minute" or "videos a minute"; "TF32" -> "t f thirty two"; "v2.1.3" -> "version
  two point one point three"; "e.g." -> "for example"; "README.md" -> "readme dot m d"; "3:15 PM"
  -> "three fifteen p m"; "$25" -> "twenty five dollars".
- Acronyms said letter by letter are written as separate letters ("GPU" -> "g p u",
  "AIGVDBench" -> "a i g v d bench", "API" -> "a p i"); acronyms said as words stay words
  ("NASA" -> "nasa", "README" -> "readme"). Mixed-case names said as words stay words
  ("WaveRep" -> "wave rep", "GitHub" -> "github").
- Write the spoken words in lowercase letters. Commas and periods may mark where the voice
  pauses; they are not spoken."""

_STYLE = {
    "natural": """Do not say any punctuation or formatting by name. Brackets, quotes, dashes,
colons and line breaks are not spoken either; the phrasing implies them.""",
    "explicit": """The speaker says punctuation and symbols by name, the way voice-typing users do:
"comma", "period" or "full stop", "question mark", "colon", "semicolon", "open paren" / "close
paren" (or "open parenthesis", "left paren" / "right paren", or just "parenthesis" for both),
"quote" / "end quote" (or "unquote"), "dash" or "em dash", "hyphen", "slash", "tilde",
"percent", "dollar sign", "at sign", "hashtag", "underscore", "new line", "new paragraph". Mix
styles naturally: most marks said, a few obvious ones (often a final period) left unsaid.""",
}


def read_prompt(style: str, script: str) -> list[dict[str, str]]:
    return [
        {"role": "system", "content": _READ + "\n\n" + _STYLE[style]},
        {"role": "user", "content": f"Text:\n{script}"},
    ]


# ---------------------------------------------------------------------------------------------
# Step 1: scripts.

_FILLERS = ("um", "uh", "um", "uh", "like", "you know", "so", "kind of")
# Cues a correction can use; "" is a bare restatement.
CUES = ("no", "no wait", "sorry", "I mean", "actually", "or rather", "wait", "")
SENTENCE_CUES = ("scratch that", "let me rephrase", "sorry, let me say that again", "actually, no")
COMMANDS = (
    "scratch that",
    "delete that",
    "never mind, let's start over",
    "let's try that from the beginning",
    "start over",
)
# Commands that retract the last sentence; the others retract everything said so far.
SENTENCE_COMMANDS = ("scratch that", "delete that")

_PROPOSE_CORRECTION = """You help build test data for a dictation app. Given a written passage,
propose one realistic self-correction a speaker might make while dictating it: they first say
something wrong, then correct themselves to the passage's actual words.

Return JSON with:
- "scope": "word", "phrase" or "sentence";
- "anchor": text copied EXACTLY from the passage (same characters, same case): one word or
  number for "word", a few words for "phrase", one whole sentence for "sentence";
- "wrong": what the speaker says first instead of the anchor: same kind of thing, plausible in
  context, clearly different (a different number, name, day, word or sentence).
Use the scope you are given; for "word", prefer numbers, names, times and key words."""

# How often each correction scope is asked for. Left to itself the LLM never chose "sentence".
SCOPES = ("word", "word", "word", "phrase", "phrase", "sentence", "sentence")

_PROPOSE_FALSE_START = """You help build test data for a dictation app. Given a written passage,
write a plausible false start: one or two short sentences on the same topic that a speaker might
begin dictating before deciding to scrap them and start over. They must not repeat the passage.
Return JSON with "false_start"."""

_PROPOSE_RESTART = """You help build test data for a dictation app. Given a written passage,
propose one false start: a speaker begins a clause one way, abandons it after a few words, and
restarts with the passage's actual words. Return JSON with:
- "anchor": two to five words copied EXACTLY from the passage, where the restart happens;
- "abandoned": two to four words the speaker says first and abandons (a different way to begin
  that clause, not the same words)."""

_CORRECTION_SCHEMA = {
    "type": "object",
    "properties": {
        "scope": {"enum": ["word", "phrase", "sentence"]},
        "anchor": {"type": "string"},
        "wrong": {"type": "string"},
    },
    "required": ["scope", "anchor", "wrong"],
}
_FALSE_START_SCHEMA = {
    "type": "object",
    "properties": {"false_start": {"type": "string"}},
    "required": ["false_start"],
}
_RESTART_SCHEMA = {
    "type": "object",
    "properties": {"anchor": {"type": "string"}, "abandoned": {"type": "string"}},
    "required": ["anchor", "abandoned"],
}


@dataclass
class Script:
    kind: str
    style: str
    text: str
    tags: dict[str, str] = field(default_factory=dict)


def _ask(llm, system: str, passage: str, schema: dict, seed: int, note: str = "") -> dict:
    user = f"Passage:\n{passage}" + (f"\n\n{note}" if note else "")
    out = llm.chat(
        [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        max_tokens=400,
        temperature=0.9,
        seed=seed,
        json_schema=schema,
    )
    return json.loads(out)


def _boundaries(passage: str) -> list[int]:
    """Offsets of word starts (not the first word), outside brackets and quotes is not
    required: a filler can fall anywhere a speaker can hesitate, which is anywhere."""
    return [m.start() for m in re.finditer(r"(?<=\s)\S", passage)]


def _insert(passage: str, at: int, piece: str) -> str:
    return passage[:at] + piece + passage[at:]


def disfluent_script(passage: str, rng: random.Random, llm, seed: int) -> Script | None:
    if len(_boundaries(passage)) < 4:
        return None
    text = passage
    tags: dict[str, str] = {}
    # The false start first, while the passage's phrases are still contiguous; then fillers.
    try:
        p = _ask(llm, _PROPOSE_RESTART, passage, _RESTART_SCHEMA, seed)
        anchor, abandoned = p["anchor"].strip(), p["abandoned"].strip()
        if (
            anchor
            and abandoned
            and passage.count(anchor) == 1
            and abandoned.lower() not in anchor.lower()
        ):
            text = _insert(text, text.index(anchor), abandoned + ", ")
            tags["false_start"] = "1"
    except (ValueError, KeyError):
        pass
    starts = _boundaries(text)
    n = max(2, min(6, len(starts) // 8))
    # Insert from the end so earlier offsets stay valid.
    picks = sorted(rng.sample(starts, min(n, len(starts))), reverse=True)
    repeats = 0
    for at in picks:
        if rng.random() < 0.35:
            word = re.match(r"\S+", text[at:])
            if word and word.group().isalpha() and len(word.group()) <= 6:
                text = _insert(text, at, word.group() + " ")
                repeats += 1
                continue
        text = _insert(text, at, rng.choice(_FILLERS) + ", ")
    tags["fillers"] = str(len(picks) - repeats)
    tags["repeats"] = str(repeats)
    return Script("disfluent", "natural", text, tags)


def correction_script(
    passage: str, rng: random.Random, llm, seed: int, force_scope: str | None = None
) -> Script | None:
    want = force_scope or rng.choice(SCOPES)
    try:
        p = _ask(llm, _PROPOSE_CORRECTION, passage, _CORRECTION_SCHEMA, seed, note=f"Scope: {want}")
    except (ValueError, KeyError):
        return None
    scope, anchor, wrong = p["scope"], p["anchor"].strip(), p["wrong"].strip()
    if (
        scope != want
        or not anchor
        or not wrong
        or passage.count(anchor) != 1
        or wrong.lower() == anchor.lower()
    ):
        return None
    idx = passage.index(anchor)
    # A sentence-scope anchor must be a whole sentence of the passage.
    if scope == "sentence" and not (
        anchor[-1] in ".!?" and (idx == 0 or passage[:idx].rstrip()[-1:] in ".!?")
    ):
        return None
    if scope == "sentence":
        cue = rng.choice(SENTENCE_CUES)
        wrong = wrong.rstrip(".!?") + "."
        piece = f"{wrong} {cue[0].upper() + cue[1:]}. "
    else:
        cue = rng.choice(CUES)
        piece = f"{wrong}, {cue}, " if cue else f"{wrong}, "
    return Script(
        "correction",
        "natural",
        _insert(passage, idx, piece),
        {"scope": scope, "cue": cue or "none"},
    )


def command_script(passage: str, rng: random.Random, llm, seed: int) -> Script | None:
    try:
        p = _ask(llm, _PROPOSE_FALSE_START, passage, _FALSE_START_SCHEMA, seed)
    except (ValueError, KeyError):
        return None
    start = p["false_start"].strip()
    if not start:
        return None
    command = rng.choice(COMMANDS)
    if command in SENTENCE_COMMANDS:
        # These retract the sentence before them, not everything (docs/cleanup-spec.md): a
        # longer false start would be partly kept, and the passage would not be the gold.
        start = re.split(r"(?<=[.!?])\s+", start)[-1]
    start = start.rstrip(".!?") + "."
    return Script(
        "command",
        "natural",
        f"{start} {command[0].upper() + command[1:]}. {passage}",
        {"command": command},
    )


def make_script(kind: str, passage: str, rng: random.Random, llm, seed: int) -> Script | None:
    if kind in ("natural", "explicit"):
        return Script(kind, kind, passage)
    if kind == "disfluent":
        return disfluent_script(passage, rng, llm, seed)
    if kind == "restate":
        s = correction_script(passage, rng, llm, seed, force_scope="sentence")
        return Script("restate", s.style, s.text, s.tags) if s else None
    if kind == "correction":
        return correction_script(passage, rng, llm, seed)
    if kind == "command":
        return command_script(passage, rng, llm, seed)
    if kind == "grammar":
        from eval.finaltext.grammar import grammar_script

        return grammar_script(passage, rng, llm, seed)
    if kind == "list":
        from eval.finaltext.lists import list_script

        return list_script(passage, rng)
    raise ValueError(kind)


# ---------------------------------------------------------------------------------------------
# Checks.

_ALPHA = re.compile(r"[a-z]+")


def _alpha_words(text: str) -> list[str]:
    """Plain words: tokens of letters only (identifiers, numbers and symbol-bearing tokens are
    read in too many ways to check word by word; the number check covers digits)."""
    words: list[str] = []
    for tok in text.lower().split():
        tok = tok.strip(".,;:!?\"'()[]{}…—–-“”‘’")
        if tok and all(c.isalpha() or c in "'-" for c in tok):
            words.extend(_ALPHA.findall(tok))
    return words


def words_in_order(script: str, spoken: str, min_ratio: float = 0.97) -> bool:
    """The script's plain words appear in the spoken text in order; a word may be read as a
    longer or shorter form of itself ("min" -> "minute")."""
    want = _alpha_words(script)
    if not want:
        return True
    got = _ALPHA.findall(spoken.lower())
    j = found = 0
    for w in want:
        k = j
        end = None
        while k < len(got) and (end := _match_at(got, k, w)) is None:
            k += 1
        if end is not None:
            found += 1
            j = end
    return found / len(want) >= min_ratio


def _match_at(got: list[str], k: int, w: str) -> int | None:
    """Where the match of `w` starting at got[k] ends (exclusive), or None. A word matches
    itself, a longer or shorter form of itself ("min" -> "minute"), or consecutive pieces that
    join up to it: letters read one by one ("gpu" -> "g p u") or a name read as words
    ("waverep" -> "wave rep", "aigvdbench" -> "a i g v d bench")."""
    joined, end = got[k], k + 1
    while joined != w and w.startswith(joined) and end < len(got):
        joined += got[end]
        end += 1
    if joined == w:
        return end
    g = got[k]
    if g.startswith(w) or (w.startswith(g) and len(g) >= 3) or _SPOKEN_AS.get(w) == g:
        return k + 1
    # A misspelled word read as the word meant ("morgage" -> "mortgage") does not match on
    # purpose: the gold keeps the typo, and a pair teaching it would teach misspelling.
    return None


# Abbreviations read as the words they stand for.
_SPOKEN_AS = {"vs": "versus"}


def same_numbers(script: str, spoken: str) -> bool:
    """The same numbers: the same digits in the same order, spoken or written."""
    return spoken_digits(script) == spoken_digits(spoken)


def check(script: str, spoken: str) -> str | None:
    """None if the spoken form is faithful to the script, else why not."""
    if not words_in_order(script, spoken):
        return "words"
    if not same_numbers(script, spoken):
        return "numbers"
    return None


# ---------------------------------------------------------------------------------------------
# Rendering.


@dataclass
class Rendition:
    passage_id: str
    kind: str
    style: str
    script: str
    spoken: str
    tags: dict[str, str]
    attempt: int
    # The model that wrote it; rows from before this field were all the local 26B QAT GGUF.
    llm: str = ""


def jobs_for(
    passages: Iterable[tuple[str, str]],
    kinds: Iterable[str],
    done: AbstractSet[tuple[str, str]] = frozenset(),
    per_passage: int = 0,
) -> list[tuple[str, str, str]]:
    """(passage id, text, kind) still to render, passage by passage: a run stopped partway
    leaves every kind equally covered, not the first kinds only.

    A restatement needs a sentence to correct and one to keep, so only passages of two or more
    sentences get one. With `per_passage`, each passage gets that many of its kinds, chosen by
    its id: a model trained on six renditions of one passage learns the passage by heart."""
    kinds = list(kinds)
    jobs = []
    for pid, text in passages:
        mine = [k for k in kinds if k != "restate" or len(_SENTENCE_ENDS.findall(text)) >= 2]
        if per_passage:
            mine = random.Random(f"kinds:{pid}").sample(mine, min(per_passage, len(mine)))
        jobs += [(pid, text, k) for k in mine if (pid, k) not in done]
    return jobs


_SENTENCE_ENDS = re.compile(r"[.!?](?:\s|$)")


def render(
    llm,
    passages: Iterable[tuple[str, str]],
    kinds: Iterable[str] = KINDS,
    tries: int = 3,
    workers: int = 4,
    seed: int = 0,
    done: AbstractSet[tuple[str, str]] = frozenset(),
    llm_name: str = "",
    per_passage: int = 0,
):
    """Yields (passage_id, kind, Rendition | None, rejections) for every passage and kind not
    already `done`. `llm` is an eval.finaltext.llm.Chat; `llm_name` is recorded per row."""
    jobs = jobs_for(passages, kinds, done, per_passage)

    def one(job: tuple[str, str, str]):
        pid, text, kind = job
        rejections: list[str] = []
        for attempt in range(tries):
            # One generator per job and attempt: reproducible whatever order threads run in.
            rng = random.Random(f"{seed}:{pid}:{kind}:{attempt}")
            script = make_script(kind, text, rng, llm, seed=attempt)
            if script is None:
                rejections.append("script")
                continue
            out = llm.chat(
                read_prompt(script.style, script.text),
                max_tokens=1024,
                temperature=0.8,
                seed=attempt,
            )
            spoken = " ".join(out.strip().split())
            why = check(script.text, spoken)
            if why is None:
                return (
                    pid,
                    kind,
                    Rendition(
                        pid, kind, script.style, script.text, spoken, script.tags, attempt, llm_name
                    ),
                    rejections,
                )
            rejections.append(why)
        return pid, kind, None, rejections

    with ThreadPoolExecutor(workers) as pool:
        yield from pool.map(one, jobs)


def to_json(r: Rendition) -> str:
    return json.dumps(asdict(r), ensure_ascii=False)


def main(
    split: str = "eval",
    model: str = "gemma4-26b-a4b-qat",
    slots: int = 6,
    limit: int = 0,
    kinds: list[str] = typer.Option(list(KINDS), "--kind", help="Kinds to render (repeatable)."),
    out_name: str = typer.Option("", help="Renditions file stem (default: renditions-<split>)."),
    llm_url: str = typer.Option(
        "", help="An OpenAI-style server to use instead of a local llama-server (e.g. vLLM)."
    ),
    per_passage: int = typer.Option(0, help="Kinds per passage, chosen by its id (0: every kind)."),
    passages_name: str = typer.Option("", help="Passages file stem (default: passages-<split>)."),
) -> None:
    """Renders every passage of a split in each kind; resumes where a previous run stopped."""
    from eval.finaltext.llm import Remote, Server, model_path
    from eval.paths import eval_dir

    root = eval_dir() / "finaltext"
    passages = [
        json.loads(line)
        for line in (root / f"{passages_name or f'passages-{split}'}.jsonl").open(encoding="utf-8")
    ]
    if limit:
        passages = passages[:limit]
    out = root / f"{out_name or f'renditions-{split}'}.jsonl"
    done: set[tuple[str, str]] = set()
    if out.exists():
        for line in out.open(encoding="utf-8"):
            r = json.loads(line)
            done.add((r["passage_id"], r["kind"]))
    failed = kept = 0
    server = Remote(llm_url) if llm_url else Server(model_path(model), slots=slots)
    with server as llm, out.open("a", encoding="utf-8") as fh:
        name = llm.served_model or model
        print(f"{len(passages)} passages, {len(done)} renditions already done, llm {name}")
        todo = [(p["id"], p["text"]) for p in passages]
        jobs = render(
            llm, todo, kinds=kinds, workers=slots, done=done, llm_name=name, per_passage=per_passage
        )
        for pid, kind, r, rejections in jobs:
            if r is None:
                failed += 1
                print(f"{pid} {kind}: no rendition passed ({rejections})", flush=True)
                continue
            fh.write(to_json(r) + "\n")
            fh.flush()
            kept += 1
            if kept % 100 == 0:
                print(f"{kept} kept, {failed} failed", flush=True)
    print(f"done: {kept} kept, {failed} failed -> {out}")


if __name__ == "__main__":
    typer.run(main)
