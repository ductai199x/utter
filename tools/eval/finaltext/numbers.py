"""The digits a text says, whether written as digits or spoken as words.

Used to check that a spoken rendition says the same numbers as its written script: both sides
are reduced to the sequence of digits they contain, in order. Spoken number words become digits
("seventy two" -> 72, "one hundred eight" -> 108, "seven point five" -> 7.5, "seven and a half"
-> 7.5, "third" -> 3); casual readings that split a number into parts give the same digit
sequence as the number ("two forty" -> 2 40 -> "240"; "twenty twenty six" -> "2026"; "three
fifteen" -> "315"). Digits already in the text are kept as they are ("v2.1", "4×", "TF32").
"""

from __future__ import annotations

import re

_UNITS = {
    "zero": 0, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7,
    "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12, "thirteen": 13,
    "fourteen": 14, "fifteen": 15, "sixteen": 16, "seventeen": 17, "eighteen": 18,
    "nineteen": 19,
}  # fmt: skip
_TENS = {
    "twenty": 20, "thirty": 30, "forty": 40, "fifty": 50, "sixty": 60, "seventy": 70,
    "eighty": 80, "ninety": 90,
}  # fmt: skip
# Decades: "the nineteen thirties" is "the 1930s".
_TENS |= {w[:-1] + "ies": v for w, v in _TENS.items()}
_SCALES = {"hundred": 100, "thousand": 1_000, "million": 1_000_000, "billion": 1_000_000_000}
_ORDINALS = {
    "first": 1, "second": 2, "third": 3, "fourth": 4, "fifth": 5, "sixth": 6, "seventh": 7,
    "eighth": 8, "ninth": 9, "tenth": 10, "eleventh": 11, "twelfth": 12, "thirteenth": 13,
    "fourteenth": 14, "fifteenth": 15, "sixteenth": 16, "seventeenth": 17, "eighteenth": 18,
    "nineteenth": 19, "twentieth": 20, "thirtieth": 30, "fortieth": 40, "fiftieth": 50,
    "sixtieth": 60, "seventieth": 70, "eightieth": 80, "ninetieth": 90, "hundredth": 100,
    "thousandth": 1000,
}  # fmt: skip
_TOKEN = re.compile(r"[a-z]+|\d+")
# Capital Roman numerals of two letters or more ("I", "V" and "X" alone are too often words).
_ROMAN_VALUES = {
    "II": 2, "III": 3, "IV": 4, "VI": 6, "VII": 7, "VIII": 8, "IX": 9, "XI": 11, "XII": 12,
    "XIII": 13, "XIV": 14, "XV": 15, "XVI": 16, "XVII": 17, "XVIII": 18, "XIX": 19, "XX": 20,
}  # fmt: skip
_ROMAN = re.compile(r"\b(?:" + "|".join(sorted(_ROMAN_VALUES, key=len, reverse=True)) + r")\b")


def _is_number_word(w: str) -> bool:
    return w in _UNITS or w in _TENS or w in _SCALES or w in _ORDINALS


class _Phrase:
    """One spoken number being read, e.g. "one hundred eight"."""

    def __init__(self) -> None:
        self.total = 0  # completed thousands-and-up groups
        self.current = 0  # the group being read (under a thousand)
        self.parts: list[str] = []  # numbers read back to back ("two forty" -> ["2", "40"])
        self.last = ""  # kind of the last word: unit, teen, tens, scale
        self.decimals = ""
        self.in_decimals = False
        self.any = False

    def _flush_part(self) -> None:
        if self.any:
            self.parts.append(str(self.total + self.current))
        self.total = self.current = 0
        self.any = False

    def add(self, w: str) -> None:
        if self.in_decimals:
            # Digit by digit ("point zero four"), or as numbers, as versions are read
            # ("sixteen point ten", "point twenty five").
            if w in _UNITS:
                v = _UNITS[w]
                if v < 10 and self.last == "tens":
                    self.decimals = self.decimals[:-1] + str(v)
                else:
                    self.decimals += str(v)
                self.last = "unit"
            elif w in _TENS:
                self.decimals += str(_TENS[w])
                self.last = "tens"
            return
        if w in _UNITS:
            v = _UNITS[w]
            # A unit right after a unit or teen starts a new number ("two four" -> 2 4);
            # after tens it completes it ("seventy two").
            if self.any and self.last in ("unit", "teen"):
                self._flush_part()
            if self.any and self.last == "tens" and v >= 10:
                self._flush_part()
            self.current += v
            self.last = "teen" if v >= 10 else "unit"
        elif w in _TENS:
            if self.any and self.last in ("unit", "teen", "tens"):
                self._flush_part()
            self.current += _TENS[w]
            self.last = "tens"
        elif w in _SCALES:
            s = _SCALES[w]
            if s == 100:
                self.current = (self.current or 1) * 100
            else:
                self.total += (self.current or 1) * s
                self.current = 0
            self.last = "scale"
        elif w in _ORDINALS:
            v = _ORDINALS[w]
            if v >= 100:
                self.current = (self.current or 1) * v
            else:
                if self.any and self.last in ("unit", "teen"):
                    self._flush_part()
                self.current += v
            self.last = "unit"
        self.any = True

    def point(self) -> None:
        self.in_decimals = True
        self.last = ""

    def half(self) -> None:
        self.in_decimals = True
        self.decimals += "5"

    def groups(self) -> list[str]:
        """The numbers read, in order; decimals are a group of their own."""
        self._flush_part()
        return self.parts + ([self.decimals] if self.decimals else [])


def spoken_digits(text: str) -> str:
    """Every number the text says, in order, as one string of digits (leading zeros of each
    number dropped)."""
    # Thousands separators join one number ("1,000"), not two.
    text = re.sub(r"(?<=\d),(?=\d{3}\b)", "", text)
    # Regnal and sequel numerals are said as numbers ("Theodosius II" -> "the second").
    text = _ROMAN.sub(lambda m: str(_ROMAN_VALUES[m.group()]), text)
    toks = _TOKEN.findall(text.lower())
    out: list[str] = []
    phrase: _Phrase | None = None
    i = 0
    while i < len(toks):
        t = toks[i]
        nxt = toks[i + 1] if i + 1 < len(toks) else ""
        if t.isdigit():
            if phrase:
                out.extend(phrase.groups())
                phrase = None
            out.append(t)
        elif _is_number_word(t) or (t == "a" and nxt in _SCALES and phrase is None):
            phrase = phrase or _Phrase()
            if t != "a":
                phrase.add(t)
        elif phrase and t == "point" and (nxt in _UNITS or nxt in _TENS or nxt == "oh"):
            phrase.point()
        elif phrase and phrase.in_decimals and t == "oh":
            phrase.decimals += "0"  # "sixteen point oh four"
        elif phrase and t == "oh" and nxt in _UNITS:
            # "nineteen oh five" is 1905; "oh" outside a number is just "oh".
            out.extend(phrase.groups())
            out.append("0")
            phrase = None
        elif phrase and t == "and" and nxt == "a" and i + 2 < len(toks) and toks[i + 2] == "half":
            phrase.half()
            i += 2
        elif phrase and t == "and" and nxt in _UNITS | _TENS and phrase.last == "scale":
            pass  # "one hundred and five"
        else:
            if phrase:
                out.extend(phrase.groups())
                phrase = None
        i += 1
    if phrase:
        out.extend(phrase.groups())
    # Leading zeros are not said ("06:25" -> "six twenty five"): drop them from every group.
    return "".join(g.lstrip("0") or "0" for g in out)
