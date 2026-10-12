from utter_tools.text.spans import Span, spans


def kinds(text: str) -> set[tuple[str, str]]:
    return {(s.text, s.kind) for s in spans(text)}


def test_numbers_symbols_terms_and_brackets():
    got = kinds("WaveRep with TF32 runs at ~240 videos/min (4× faster) on the GPU.")
    assert ("WaveRep", "term") in got
    assert ("TF32", "number") in got and ("TF32", "term") in got
    assert ("~240", "number") in got and ("~240", "symbol") in got
    assert ("videos/min", "symbol") in got
    assert ("4×", "number") in got and ("4×", "symbol") in got
    assert ("(4× faster)", "bracket") in got
    assert ("GPU", "term") in got


def test_ordinary_words_are_not_spans():
    assert spans("I think we should ship it on Friday, okay?") == []


def test_quotes_are_spans_with_their_marks():
    assert Span('"boot into it"', "quote") in spans('When you say "boot into it" you mean the GUI?')
