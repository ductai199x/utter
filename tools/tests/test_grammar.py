import random

import pytest

pytest.importorskip("errant")

from utter_tools.data.grammar import PROFILES, TYPES, check, edit_types, plan


def test_requested_errors_pass():
    right = "Yesterday he and I went to the store, and they were out of eggs."
    wrong = "Yesterday me and him went to the store, and they was out of eggs."
    assert check(wrong, right, ["PRON", "PRON", "VERB:SVA"]) is None


def test_an_edit_of_another_kind_is_rejected():
    right = "She bought a new car last week."
    wrong = "She buy a new truck last week."  # "truck" changes the meaning, not the grammar
    assert check(wrong, right, ["VERB:TENSE"]) is not None


def test_too_many_edits_are_rejected():
    right = "The results are in the table below."
    wrong = "Result is on table in below."
    assert check(wrong, right, ["DET"]) is not None


def test_far_fewer_errors_than_asked_is_rejected():
    right = "OK, what do I put in the bug report? That I installed the package."
    wrong = "OK, what do I put in bug report? That I installed the package."
    assert check(wrong, right, ["DET", "VERB:TENSE", "DET", "DET"]) == "too few edits"


def test_no_change_is_rejected():
    s = "The results are in the table below."
    assert check(s, s, ["DET"]) == "no errors"


def test_punctuation_and_case_are_not_grammar():
    assert (
        edit_types("the results are in the table below", "The results are in the table below.")
        == []
    )


def test_plans_draw_types_from_their_profile():
    rng = random.Random(0)
    for _ in range(50):
        profile, _level, requested = plan("word " * 40, rng)
        assert requested and all(t in PROFILES[profile] for t in requested)
        assert all(t in TYPES for t in requested)


def test_gold_grammar_fixes_apply_and_rewording_does_not():
    from utter_tools.data.goldfix import apply_grammar

    text = "Why dont you come? me and him goes there every week on the desktop."
    fixes = [
        {"kind": "apostrophe", "wrong": "dont", "right": "don't"},
        {"kind": "grammar", "wrong": "me and him goes", "right": "He and I go"},
        {"kind": "grammar", "wrong": "desktop", "right": "laptop"},
    ]
    out = apply_grammar(text, fixes)
    assert out.text == "Why don't you come? He and I go there every week on the desktop."
