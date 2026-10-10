from eval.finaltext.trainset import BEFORE_CHARS, Context, user_message


def test_user_message_matches_the_rust_client_exactly():
    # The same cases as crates/utter-llm/src/cleanup.rs tests: training and serving must send
    # byte-identical prompts.
    assert (
        user_message(" hello there ", ["Utter", "Nguyen"])
        == "Dictionary: Utter, Nguyen\nDictated: hello there"
    )
    assert user_message("hi", []) == "Dictated: hi"


def test_context_lines_match_the_rust_client_exactly():
    ctx = Context(
        app="email",
        single_line=True,
        before='Hi Sam,\nI think we "should"',
        after=" today.",
        lists_off=True,
    )
    assert user_message("move it", ["Utter"], ctx) == (
        'App: email\nField: single-line\nBefore: "Hi Sam,\\nI think we \\"should\\""\n'
        'After: " today."\nLists: off\nDictionary: Utter\nDictated: move it'
    )


def test_only_the_end_of_the_text_before_the_cursor_is_sent():
    ctx = Context(before="x" * (BEFORE_CHARS + 10) + "end")
    line = user_message("hi", [], ctx).split("\n")[0]
    assert line.endswith('end"')
    assert len(line) == len('Before: ""') + BEFORE_CHARS
