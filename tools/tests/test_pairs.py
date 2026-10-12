from utter_tools.data.lists import parse, prose_gold
from utter_tools.data.pairs import TermPool, build, user_message


def clip(rid: str, category: str, final_text: str) -> dict:
    return {
        "id": f"corpus-x/{rid}",
        "rendition_id": rid,
        "category": category,
        "domain": "list" if category == "list" else "everyday-text",
        "final_text": final_text,
        "annotations": ["variant=plain", "ordered=False"],
        "source": "synthetic",
        "source_url": "https://example.org",
        "license": "cc-by-sa-4.0",
    }


def test_rows_are_the_chat_the_app_sends():
    clips = [clip("0123456789ab-natural", "natural", "Ship it on Friday.")]
    heard = {"corpus-x/0123456789ab-natural": "ship it on friday"}
    [r] = build(clips, heard, TermPool([]), seed=0, dictionary_share=0.0)
    assert r["id"] == "0123456789ab-natural"
    assert r["dictated"] == "ship it on friday"
    assert [m["role"] for m in r["messages"]] == ["system", "user", "assistant"]
    assert r["messages"][1]["content"] == "Dictated: ship it on friday"
    assert r["messages"][2]["content"] == r["final_text"] == "Ship it on Friday."


def test_lists_off_rows_get_the_prose_gold():
    gold = "Groceries:\n- Eggs\n- Milk\n- Bread"
    prose = prose_gold(parse(gold), "plain")
    clips = [clip(f"{i:012x}-list", "list", gold) for i in range(40)]
    heard = {c["id"]: f"groceries eggs milk and bread {i}" for i, c in enumerate(clips)}
    rows = build(clips, heard, TermPool([]), seed=0, lists_off_share=0.5, dictionary_share=0.0)
    off = [r for r in rows if not r["lists"]]
    assert 0 < len(off) < len(rows)
    assert {r["final_text"] for r in off} == {prose}
    assert all(r["messages"][1]["content"].startswith("Lists: off\n") for r in off)
    assert all(r["final_text"] == gold for r in rows if r["lists"])


def test_clips_heard_alike_make_one_row_and_silence_none():
    clips = [
        clip("0123456789ab-natural", "natural", "Hi."),
        clip("0123456789ab-explicit", "explicit", "Hi."),
    ]
    heard = {"corpus-x/0123456789ab-natural": "hi", "corpus-x/0123456789ab-explicit": "hi"}
    assert [r["id"] for r in build(clips, heard, TermPool([]), seed=0)] == ["0123456789ab-natural"]
    assert build(clips, {}, TermPool([]), seed=0) == []


def test_user_message_matches_the_rust_client_exactly():
    # The same cases as crates/utter-llm/src/cleanup.rs tests: training and serving must send
    # byte-identical prompts.
    assert (
        user_message(" hello there ", ["Utter", "Nguyen"])
        == "Dictionary: Utter, Nguyen\nDictated: hello there"
    )
    assert user_message("hi", []) == "Dictated: hi"


def test_lists_off_comes_before_the_dictionary_as_in_the_rust_client():
    assert user_message("move it", ["Utter"], lists_off=True) == (
        "Lists: off\nDictionary: Utter\nDictated: move it"
    )
