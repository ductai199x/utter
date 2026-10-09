"""Text normalization for word error rate, so a WER counts words, not formatting.

English uses Whisper's English normalizer, the one the Open ASR Leaderboard scores with
(spelling variants, numbers, contractions). Vietnamese keeps its diacritics, which carry
meaning, and drops case and punctuation: the references disagree on both (GigaSpeech2 is all
capitals with no punctuation; FLEURS is cased and punctuated).

Known limit: a number written as digits in the reference and as words in the output (or the
reverse) counts as an error in Vietnamese. It costs every model alike.
"""

from __future__ import annotations

import re
import unicodedata
from functools import cache


@cache
def _english():
    from transformers import WhisperTokenizer

    tok = WhisperTokenizer.from_pretrained("openai/whisper-large-v3-turbo")
    return tok.normalize


_PUNCT = re.compile(r"[^\w\s]", re.UNICODE)
_SPACE = re.compile(r"\s+")


def vietnamese(text: str) -> str:
    text = unicodedata.normalize("NFC", text).lower()
    text = _PUNCT.sub(" ", text)
    return _SPACE.sub(" ", text).strip()


def normalize(text: str, lang: str) -> str:
    if lang == "en":
        return _SPACE.sub(" ", _english()(text)).strip()
    return vietnamese(text)
