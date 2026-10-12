"""The standard cleanup eval: which sets, through which recognizer run, with which dictionary, and
which are graded for ready to send. `utter-eval run` follows it and `utter-eval report` reads it.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from utter_tools.eval.dictionary import DICTIONARIES
from utter_tools.eval.sets import DICTATED, TEXT_SET

# Utter's recognizer through the Rust runtime (utter-cli bench), plain and with phrase boosting.
ASR = "parakeet"
ASR_BOOSTED = "parakeet-dict"
# Sets also run with a user's dictionary: boosted in the recognizer, sent to cleanup.
DICTIONARY_SETS = {
    "kokoro-real": DICTIONARIES / "kokoro-real-300.txt",
    "kokoro-directed": DICTIONARIES / "kokoro-directed-231.txt",
}
# Graded for ready to send (the headline); the rest have strict scores only.
GRADED = {TEXT_SET, "kokoro-real", "chatterbox-real", "kokoro-directed"}
# The order sets are reported in: the headline sets first.
ORDER = [TEXT_SET, "kokoro-real", "chatterbox-real", "kokoro-directed"]


@dataclass(frozen=True)
class Item:
    eval_set: str
    asr_run: str
    # The cleanup run's name is the model's plus this ("-dict" with a dictionary).
    suffix: str = ""
    dictionary: Path | None = None

    @property
    def graded(self) -> bool:
        return self.eval_set in GRADED

    def label(self) -> str:
        return f"{self.eval_set} + dictionary" if self.dictionary else self.eval_set


def items(speech_sets: list[str]) -> list[Item]:
    ordered = [s for s in ORDER if s in speech_sets] + sorted(set(speech_sets) - set(ORDER))
    out = [Item(TEXT_SET, DICTATED)]
    for s in ordered:
        out.append(Item(s, ASR))
        if s in DICTIONARY_SETS:
            out.append(Item(s, ASR_BOOSTED, "-dict", DICTIONARY_SETS[s]))
    return out
