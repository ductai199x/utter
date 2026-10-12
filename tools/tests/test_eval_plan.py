from utter_tools.eval import plan
from utter_tools.eval.sets import DICTATED, TEXT_SET


def test_the_headline_sets_come_first_and_dictionary_runs_follow_their_set():
    speech = ["kokoro-lists", "chatterbox-real", "kokoro-real", "kokoro-directed", "kokoro"]
    items = plan.items(speech)
    assert [i.label() for i in items] == [
        TEXT_SET,
        "kokoro-real",
        "kokoro-real + dictionary",
        "chatterbox-real",
        "kokoro-directed",
        "kokoro-directed + dictionary",
        "kokoro",
        "kokoro-lists",
    ]
    assert items[0].asr_run == DICTATED
    boosted = [i for i in items if i.dictionary]
    assert all(i.asr_run == plan.ASR_BOOSTED and i.suffix == "-dict" for i in boosted)
    assert all(i.dictionary.is_file() for i in boosted)


def test_graded_sets_are_the_headline_ones():
    graded = {
        i.label() for i in plan.items(["kokoro-real", "kokoro", "chatterbox-real"]) if i.graded
    }
    assert graded == {TEXT_SET, "kokoro-real", "kokoro-real + dictionary", "chatterbox-real"}
