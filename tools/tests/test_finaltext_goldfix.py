from eval.finaltext.goldfix import apply


def test_fixes_apply_where_they_match_once():
    o = apply(
        "I installed ubuntu and it was a big hasle.",
        False,
        [
            {"kind": "capitalization", "wrong": "ubuntu", "right": "Ubuntu"},
            {"kind": "typo", "wrong": "hasle", "right": "hassle"},
        ],
        spoken_safe=False,
    )
    assert o.text == "I installed Ubuntu and it was a big hassle."
    assert len(o.applied) == 2


def test_ambiguous_fixes_are_skipped():
    o = apply(
        "the cat and the dog", False, [{"kind": "typo", "wrong": "the", "right": "The"}], False
    )
    assert o.text == "the cat and the dog"
    assert o.applied == []


def test_not_prose_is_dropped():
    assert apply("Smith, J. (1999). Title. Journal 4(2).", True, [], False).text is None


def test_too_many_fixes_drop_the_passage():
    fixes = [{"kind": "typo", "wrong": w, "right": w + "x"} for w in ("aaaa", "bbbb", "cccc")]
    assert apply("aaaa bbbb cccc dddd", False, fixes, False).text is None


def test_spoken_safe_allows_case_only():
    ok = apply(
        "I use ubuntu daily.",
        False,
        [{"kind": "capitalization", "wrong": "ubuntu", "right": "Ubuntu"}],
        True,
    )
    assert ok.text == "I use Ubuntu daily."
    bad = apply(
        "I d/l it daily.", False, [{"kind": "shorthand", "wrong": "d/l", "right": "download"}], True
    )
    assert bad.text is None


def test_fixes_match_whole_words_only():
    o = apply(
        "Our research is by the sea.",
        False,
        [{"kind": "capitalization", "wrong": "sea", "right": "Sea"}],
        False,
    )
    assert o.text == "Our research is by the Sea."
    inside = apply(
        "Our research team.",
        False,
        [{"kind": "capitalization", "wrong": "sea", "right": "Sea"}],
        False,
    )
    assert inside.text == "Our research team."
