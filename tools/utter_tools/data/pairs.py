"""Text rows (dictation-to-text's schema) from a batch's transcribed clips: what Parakeet heard
(its actual output on the speech, errors and habits included) and the text that was meant, in the
exact chat the app sends (crates/utter-llm/src/cleanup.rs, messages_with):

- a system prompt (the fine-tuned models' prompt, crates/utter-llm/prompts/cleanup-ft.md);
- a user message: "Lists: off" when lists are off, "Dictionary: ..." with the dictionary terms
  heard in the dictation (dictionary.relevant_terms, the mirror of gate::relevant_terms), then
  "Dictated: <transcript>";
- the final text as the assistant's answer.

Lists off: a share of list renditions is trained with lists off and the list said as prose
(lists.prose_gold), as a user who turned lists off would get it.

Dictionaries: a share of rows gets a user's dictionary: some of the terms the speaker meant
(names, product names, acronyms: text.terms) among many unrelated ones from a pool of all the
rows' terms, plus sometimes a term close to a word said that was not meant ("Thorne" for
"there"). Only the terms heard go into the message, as the app sends them; a row where none is
heard has no dictionary. The model learns to spell a term the dictionary's way when it was said,
to leave a word alone when the term was not meant, and never to type the line.
"""

from __future__ import annotations

import random
from collections.abc import Iterable

from rapidfuzz import process
from rapidfuzz.distance import Levenshtein

from utter_tools import config, schema
from utter_tools.data.lists import parse, prose_gold
from utter_tools.text.dictionary import relevant_terms
from utter_tools.text.terms import candidate_terms, lowercase_counts

SYSTEM_PROMPT = config.REPO / "crates" / "utter-llm" / "prompts" / "cleanup-ft.md"


def user_message(transcript: str, dictionary: list[str], lists_off: bool = False) -> str:
    """Mirrors cleanup::messages_with in Rust, byte for byte, for what the app sends: the lists
    setting and the dictionary terms heard (cleanup_context in crates/utter-pipeline)."""
    out = "Lists: off\n" if lists_off else ""
    if dictionary:
        out += "Dictionary: " + ", ".join(dictionary) + "\n"
    return out + "Dictated: " + transcript.strip()


def text_row(
    clip: dict, heard: str, final_text: str, lists: bool, dictionary: list[str], system: str
) -> dict:
    """A text row for a clip (a speech row) and what the recognizer heard."""
    user = user_message(heard, dictionary, lists_off=not lists)
    return {
        "id": clip["rendition_id"],
        "category": clip["category"],
        "domain": clip["domain"],
        "dictated": heard,
        "final_text": final_text,
        "dictionary": dictionary,
        "lists": lists,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": final_text},
        ],
        **{k: clip[k] for k in schema.PROVENANCE},
    }


def _letters(s: str) -> str:
    return "".join(c for c in s.lower() if c.isalnum())


class TermPool:
    """Every term the rows could hold, for drawing users' dictionaries."""

    def __init__(self, golds: Iterable[str]) -> None:
        golds = list(golds)
        self.lower = lowercase_counts(golds)
        self.terms = sorted({t for g in golds for t in candidate_terms(g, self.lower)})
        self.by_letters: dict[str, list[str]] = {}
        for t in self.terms:
            self.by_letters.setdefault(_letters(t), []).append(t)
        self.keys = list(self.by_letters)

    def near(self, transcript: str) -> list[str]:
        """Terms one edit from a word of five letters or more in the transcript."""
        out: list[str] = []
        for w in {_letters(w) for w in transcript.split()}:
            if len(w) < 5:
                continue
            for key, _, _ in process.extract_iter(
                w, self.keys, scorer=Levenshtein.distance, score_cutoff=1
            ):
                if key != w:
                    out += self.by_letters[key]
        return out

    def dictionary(self, transcript: str, gold: str, rng: random.Random) -> list[str]:
        """A user's dictionary for this dictation, reduced to the terms heard in it."""
        mine = sorted(candidate_terms(gold, self.lower))
        mine = rng.sample(mine, min(len(mine), rng.randint(1, 3))) if mine else []
        dictionary = mine + rng.sample(self.terms, min(len(self.terms), rng.randint(10, 150)))
        # In real use, a heard term is often not meant: a name close to a word said. Random terms
        # rarely are, so some are picked to be.
        if rng.random() < 0.5:
            near = [t for t in self.near(transcript) if t not in gold]
            dictionary += rng.sample(near, min(len(near), 2))
        rng.shuffle(dictionary)
        return relevant_terms(transcript, list(dict.fromkeys(dictionary)))


def build(
    clips: list[dict],
    heard: dict[str, str],
    pool: TermPool,
    seed: int,
    lists_off_share: float = 0.2,
    dictionary_share: float = 0.5,
) -> list[dict]:
    """Text rows for the clips the recognizer heard something in (`heard`: speech row id ->
    transcript). Two clips heard identically with the same final text make one row (the first)."""
    system = SYSTEM_PROMPT.read_text(encoding="utf-8")
    rng = random.Random(seed)
    rows, pairs = [], set()
    for clip in clips:
        hyp = heard.get(clip["id"], "").strip()
        if not hyp:
            continue
        final, lists = clip["final_text"], True
        if clip["category"] == "list" and rng.random() < lists_off_share:
            lp = parse(final)
            if lp is not None:
                final, lists = (
                    prose_gold(lp, schema.from_annotations(clip["annotations"])["variant"]),
                    False,
                )
        if (hyp, final) in pairs:
            continue
        pairs.add((hyp, final))
        dictionary = pool.dictionary(hyp, final, rng) if rng.random() < dictionary_share else []
        rows.append(text_row(clip, hyp, final, lists, dictionary, system))
    return rows
