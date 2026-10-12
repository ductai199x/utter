import json
import random

from utter_tools.data.amend import amend_script
from utter_tools.data.lists import ListPassage


class FakeLLM:
    def __init__(self, reply: dict) -> None:
        self.reply = json.dumps(reply)

    def chat(self, *args, **kwargs) -> str:
        return self.reply


def first_with(op: str, passage: str, llm) -> str:
    """The script of the first seed whose operation is `op`."""
    for seed in range(200):
        s = amend_script(passage, random.Random(seed), llm, seed)
        if s and s.tags["op"] == op:
            return s.text
    raise AssertionError(f"no {op} script")


def test_a_value_said_wrong_is_fixed_later_in_prose():
    passage = "Let's meet on Thursday at 3 PM in room 204. I'll bring the slides."
    text = first_with("value", passage, FakeLLM({"target": "Thursday", "wrong": "Tuesday"}))
    assert text.startswith("Let's meet on Tuesday at 3 PM in room 204.")
    assert "Tuesday" in text.split("204.", 1)[1] and "Thursday" in text


def test_list_edits_remove_insert_and_replace_by_position():
    groceries = ListPassage("Groceries:", ["eggs", "milk", "bread", "coffee"], False).markdown()
    removed = first_with("delete", groceries, FakeLLM({"extra": "an umbrella"}))
    assert "an umbrella" in removed.split(".")[0] or "an umbrella" in removed
    assert removed.rstrip(".").endswith(
        ("an umbrella off the list", "an umbrella", "umbrella from that")
    )

    inserted = first_with("insert", groceries, FakeLLM({}))
    assert "after" in inserted.rsplit(".", 2)[-2]

    by_position = first_with("position", groceries, FakeLLM({"target": "x", "wrong": "oat milk"}))
    assert "oat milk" in by_position and ("one " in by_position or "item to" in by_position)


def test_a_wrong_version_already_in_the_passage_is_rejected():
    passage = "Bring Tuesday's report on Thursday."
    llm = FakeLLM({"target": "Thursday", "wrong": "Tuesday"})
    assert all(amend_script(passage, random.Random(s), llm, s) is None for s in range(20))
