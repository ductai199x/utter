from eval.finaltext.trainset import user_message


def test_user_message_matches_the_rust_client_exactly():
    # The same cases as crates/utter-llm/src/cleanup.rs tests: training and serving must send
    # byte-identical prompts.
    assert (
        user_message(" hello there ", ["Utter", "Nguyen"])
        == "Dictionary: Utter, Nguyen\nDictated: hello there"
    )
    assert user_message("hi", []) == "Dictated: hi"
