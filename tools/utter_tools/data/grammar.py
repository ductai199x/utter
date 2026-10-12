"""Grammar errors for the "grammar" rendition kind: a correct passage is rewritten the way a
speaker who makes grammar mistakes would say it, so the cleanup model learns to restore correct
English (docs/cleanup-spec.md, rule 8; docs/plan-use-cases.md, recipe R2).

The method follows tagged corruption (Stahlberg & Kumar 2021, the C4_200M data): errors are
requested by type, with the types drawn from a speaker profile (the first language and the
habits it carries into English), and ERRANT checks that the realized edits are of those types.
A rewrite that changes anything else (a different noun, a new word, a dropped clause) is
rejected, because its gold would then be a different text, not a corrected one. The LLM only
proposes; code decides.
"""

from __future__ import annotations

import random
import re
from functools import cache

from utter_tools.data.script import Script, ask, same_numbers
from utter_tools.llm import Chat

# ERRANT error types a speaker produces (spelling, orthography and punctuation are left out: a
# recognizer cannot misspell, and Parakeet punctuates). Each with how the LLM is asked for it.
TYPES = {
    "DET": 'articles: a/an/the missing, extra or wrong ("I bought car", "the life is hard")',
    "PREP": 'prepositions: wrong or missing ("since three weeks", "depends of", "in Monday")',
    "VERB:TENSE": 'tense or aspect ("yesterday I go", "I am knowing him", "I live here since 2019")',
    "VERB:SVA": 'subject-verb agreement ("he go", "they was", "the list of items are")',
    "VERB:FORM": 'verb form ("I have went", "can sending", "we buyed", "I seen it")',
    "VERB": 'a missing auxiliary or copula ("he going home", "she very tired")',
    "NOUN:NUM": 'noun number and countability ("two book", "many informations", "one of my friend")',
    "PRON": 'pronoun case or choice ("me and him went", "he" for "she", "who" for "which")',
    "ADJ:FORM": 'comparatives and superlatives ("more better", "the most fast", "bigger as")',
    "ADV": 'an adjective used as an adverb, or a misplaced adverb ("runs quick", "real good")',
    "WO": 'word order, including questions ("what you are doing?", "always I go")',
    "PART": 'particles ("look for it up", "turn on it")',
    "CONJ": 'conjunctions ("although it rained, but we went", "because of it rained")',
    "OTHER": 'a double negative ("I don\'t have no time")',
}

# The ERRANT types a requested type may come out as: the same error is often labeled a sibling
# type ("buyed" is VERB:FORM or VERB:INFL, a missing "is" is VERB or VERB:FORM).
ACCEPT = {
    "DET": {"DET"},
    "PREP": {"PREP", "PART"},
    "VERB:TENSE": {"VERB:TENSE", "VERB", "VERB:FORM"},
    "VERB:SVA": {"VERB:SVA", "VERB:FORM", "VERB:INFL", "VERB:TENSE"},
    "VERB:FORM": {"VERB:FORM", "VERB:INFL", "VERB:TENSE", "VERB"},
    "VERB": {"VERB", "VERB:FORM", "VERB:TENSE"},
    "NOUN:NUM": {"NOUN:NUM", "NOUN:INFL", "DET"},
    "PRON": {"PRON"},
    "ADJ:FORM": {"ADJ:FORM", "ADJ", "ADV"},
    "ADV": {"ADV", "ADJ", "WO"},
    "WO": {"WO", "VERB", "VERB:FORM"},
    "PART": {"PART", "PREP"},
    "CONJ": {"CONJ", "PREP"},
    "OTHER": {"OTHER", "DET", "ADV", "PRON"},
}
# Edits that change nothing a listener hears as grammar: ignored when checking.
_IGNORED = {"PUNCT", "ORTH", "CONTR"}

# Speaker profiles: which errors a speaker carries into English, weighted. Native casual speech
# has its own (nonstandard pronouns, agreement, past forms).
PROFILES = {
    "native-casual": {"PRON": 3, "VERB:SVA": 3, "VERB:FORM": 3, "ADV": 2, "OTHER": 1},
    "spanish": {
        "DET": 3,
        "PREP": 3,
        "VERB:SVA": 2,
        "NOUN:NUM": 1,
        "WO": 1,
        "VERB:TENSE": 1,
        "PRON": 1,
    },
    "chinese": {"DET": 4, "VERB:TENSE": 3, "NOUN:NUM": 3, "VERB:SVA": 2, "PREP": 2, "VERB": 2},
    "hindi": {"DET": 3, "PREP": 3, "VERB:TENSE": 3, "WO": 2, "PRON": 1, "VERB:SVA": 1},
    "vietnamese": {"VERB:TENSE": 3, "NOUN:NUM": 3, "DET": 3, "VERB:SVA": 2, "VERB": 2, "PREP": 1},
    "arabic": {"DET": 3, "VERB": 3, "PREP": 3, "VERB:TENSE": 2, "VERB:SVA": 1},
    "russian": {"DET": 4, "PREP": 3, "VERB:TENSE": 2, "WO": 2, "NOUN:NUM": 1},
    "korean-japanese": {"DET": 4, "NOUN:NUM": 3, "PREP": 2, "VERB:SVA": 2, "PRON": 1, "WO": 1},
    "mixed": {t: 1 for t in TYPES},
}
# How many errors, by level: a fixed range, or one per so many words for broken English.
LEVELS = {"light": (1, 2), "medium": (3, 4), "heavy": None}

_PROPOSE = """You help build training data for a dictation app that turns a speaker's
ungrammatical English into correct English. Given a correct passage, rewrite it the way this
speaker would say it, making grammar mistakes of exactly the requested kinds.

Rules:
- Make exactly the number of mistakes asked for, each of one of the requested kinds, spread over
  the passage. Each mistake changes one word or a few adjacent words.
- The mistakes must be natural for this speaker: the kind a real person with this background
  makes when speaking.
- Change nothing else: keep every other word, the word order, names, numbers and punctuation
  exactly as they are. Do not change vocabulary, add ideas or drop any.
Return JSON with "text": the passage with the mistakes."""

_SCHEMA = {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}


@cache
def annotator():
    import errant

    return errant.load("en")


def edit_types(wrong: str, right: str) -> list[str]:
    """ERRANT's types for the edits turning `wrong` into `right` ("VERB:SVA", "DET", ...),
    leaving out punctuation, case and contraction edits."""
    a = annotator()
    edits = a.annotate(a.parse(wrong, tokenise=True), a.parse(right, tokenise=True))
    types = [e.type.split(":", 1)[1] for e in edits if e.type not in ("noop", "UNK")]
    return [t for t in types if t not in _IGNORED]


def check(wrong: str, right: str, requested: list[str]) -> str | None:
    """None if `wrong` differs from `right` only by about the requested number of errors, all of
    the requested kinds; otherwise why not."""
    types = edit_types(wrong, right)
    if not types:
        return "no errors"
    if len(types) > len(requested) + 1:
        return "too many edits"
    # A level asked for is a level delivered: "heavy" with one mistake is "light".
    if len(types) < (len(requested) + 1) // 2:
        return "too few edits"
    allowed = set().union(*(ACCEPT[t] for t in requested))
    stray = [t for t in types if t not in allowed]
    if stray:
        return "other edits: " + ",".join(sorted(set(stray)))
    return None


def plan(passage: str, rng: random.Random) -> tuple[str, str, list[str]]:
    """A speaker profile, a level and the error types to make, for this passage."""
    profile = rng.choice(list(PROFILES))
    level = rng.choices(list(LEVELS), weights=[5, 3, 2])[0]
    words = len(passage.split())
    span = LEVELS[level]
    n = rng.randint(*span) if span else max(3, min(10, words // 8))
    n = min(n, max(1, words // 5))
    weights = {t: w * BOOST.get(t, 1.0) for t, w in PROFILES[profile].items()}
    requested = rng.choices(list(weights), weights=list(weights.values()), k=n)
    return profile, level, requested


# Extra weight per error type, set from the command line (utter-data render --grammar-boost) to
# aim a training round at the types a model still misses (the grammar eval, broken down by type).
BOOST: dict[str, float] = {}


def grammar_script(passage: str, rng: random.Random, llm: Chat, seed: int) -> Script | None:
    """A grammar rendition script, or None when the proposal fails the check."""
    profile, level, requested = plan(passage, rng)
    kinds = sorted(set(requested))
    note = (
        f"Speaker: {profile.replace('-', ' ')} speaker of English.\n"
        f"Make {len(requested)} mistakes, of these kinds:\n"
        + "\n".join(f"- {TYPES[k]}" for k in kinds)
    )
    p = ask(llm, _PROPOSE, f"Passage:\n{passage}\n\n{note}", _SCHEMA, seed, max_tokens=1024)
    wrong = " ".join(p.get("text", "").split()) if p else ""
    if not wrong or wrong == passage or not same_numbers(passage, wrong):
        return None
    # Proper names and terms must survive: their spelling is not a grammar matter.
    keep = set(re.findall(r"\b[A-Z][\w'-]*[A-Z0-9][\w'-]*\b", passage))
    if any(k not in wrong for k in keep):
        return None
    if check(wrong, passage, requested) is not None:
        return None
    tags = {"profile": profile, "level": level, "errors": ",".join(requested)}
    return Script("grammar", "natural", wrong, tags)
