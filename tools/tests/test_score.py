import re

from utter_tools.text.score import marks, parse, prf, score


def simple(text: str) -> str:
    return re.sub(r"[^\w\s]", " ", text.lower())


def test_parse_splits_words_from_their_gaps():
    p = parse('He said "go" (now); 4× faster, ~240 videos/min.')
    assert [w.surface for w in p.words] == [
        "He",
        "said",
        "go",
        "now",
        "4×",
        "faster",
        "~240",
        "videos/min",
    ]
    assert p.gaps == ["", " ", ' "', '" (', "); ", " ", ", ", " ", "."]


def test_words_keep_inner_marks():
    p = parse("don't use protocol-eligible e.g. 3.5")
    assert [w.surface for w in p.words] == ["don't", "use", "protocol-eligible", "e.g", "3.5"]


def test_marks_normalize_quote_and_dash_styles():
    assert marks(" “x") == marks(' "x')
    assert marks(" — ") == marks(" - ") == {"—": 1}
    assert marks(".\n\n") == {".": 1, "\n\n": 1}


def test_identical_text_is_exact():
    s = score("Let's meet at 4:30.", "Let's  meet at 4:30.", normalize=simple)
    assert s.exact
    assert s.chars_to_fix == 0
    assert s.substitutions == s.deletions == s.insertions == 0


def test_a_missing_comma_is_a_false_negative_for_comma_only():
    s = score("First, we ship it.", "First we ship it.", normalize=simple)
    assert not s.exact
    assert s.marks[","] == [0, 0, 1]
    assert s.marks["."] == [1, 0, 0]
    assert s.substitutions == s.deletions == s.insertions == 0


def test_brackets_and_quotes_are_scored_where_they_sit():
    ref = "It runs at ~240 videos/min (4× faster)."
    hyp = "It runs at ~240 videos/min, 4× faster."
    s = score(ref, hyp, normalize=simple)
    assert s.marks["("] == [0, 0, 1]
    assert s.marks[")"] == [0, 0, 1]
    assert s.marks[","] == [0, 1, 0]
    assert s.marks["."] == [1, 0, 0]


def test_a_deleted_word_does_not_also_cost_its_punctuation():
    s = score("We ship it today.", "We ship today.", normalize=simple)
    assert s.deletions == 1
    assert s.marks["."] == [1, 0, 0]


def test_casing_counts_only_words_with_the_same_letters():
    s = score("Send it to Sarah by Friday.", "send it to sarah by friday.", normalize=simple)
    assert s.casing_checked == 6
    assert s.casing_wrong == 3


def test_spans_must_appear_verbatim():
    s = score(
        "Use TF32 (4× faster).",
        "Use TF 32 (4× faster).",
        spans=["TF32", "(4× faster)"],
        normalize=simple,
    )
    assert s.spans_total == 2
    assert s.spans_hit == 1
    assert s.missed_spans == ["TF32"]


def test_paragraph_breaks_are_marks():
    s = score("Hi team,\n\nThanks.", "Hi team, Thanks.", normalize=simple)
    assert s.marks["\n\n"] == [0, 0, 1]
    assert s.casing_wrong == 0


def test_prf_handles_empty_classes():
    assert prf([0, 0, 0]) == (1.0, 1.0, 1.0)
    assert prf([1, 1, 0])[0] == 0.5
