import random

from eval.finaltext.spoken import (
    COMMANDS,
    SCOPES,
    check,
    command_script,
    correction_script,
    disfluent_script,
    same_numbers,
    words_in_order,
)


def test_words_match_letter_by_letter_and_split_names():
    script = "AIGVDBench on the GPU, says WaveRep."
    assert words_in_order(script, "a i g v d bench on the g p u, says wave rep.")


def test_words_accept_longer_and_shorter_forms():
    assert words_in_order("videos per min", "videos per minute")


def test_abbreviations_match_the_words_read():
    assert words_in_order("the UK loan vs a mortgage", "the u k loan versus a mortgage", 1.0)


def test_a_typo_read_as_the_word_meant_does_not_match():
    # The gold keeps the typo; a pair from this rendition would teach misspelling.
    assert not words_in_order("a UK morgage", "a u k mortgage", min_ratio=1.0)


def test_dropped_words_fail():
    assert not words_in_order(
        "send the report to the platform team by friday",
        "send the report to the team by friday",
        min_ratio=1.0,
    )


def test_numbers_must_be_the_same_digits_in_order():
    assert same_numbers("all 108k in 7.5 hours", "all 108 k in 7.5 hours")
    assert not same_numbers("all 108k in 7.5 hours", "all 188 k in 7.5 hours")


def test_check_reports_why():
    assert check("at 4 pm", "at 5 pm") == "numbers"
    assert check("send it to sam", "send it") == "words"
    assert check("send it to sam", "send it to sam") is None


class FakeLLM:
    """Answers the structured proposals with fixed JSON."""

    def __init__(self, reply: str) -> None:
        self.reply = reply

    def chat(self, *args, **kwargs) -> str:
        return self.reply


def rng_asking_for(scope: str) -> random.Random:
    """A generator whose first scope choice is `scope`."""
    seed = next(s for s in range(1000) if random.Random(s).choice(SCOPES) == scope)
    return random.Random(seed)


def rng_choosing(command: str) -> random.Random:
    """A generator whose first command choice is `command`."""
    seed = next(s for s in range(1000) if random.Random(s).choice(COMMANDS) == command)
    return random.Random(seed)


def test_scratch_that_follows_a_single_sentence():
    llm = FakeLLM('{"false_start": "The build is broken. We should roll back."}')
    s = command_script("Ship it on Friday.", rng_choosing("scratch that"), llm, seed=0)
    assert s is not None
    assert s.text == "We should roll back. Scratch that. Ship it on Friday."


def test_start_over_keeps_the_whole_false_start():
    llm = FakeLLM('{"false_start": "The build is broken. We should roll back."}')
    s = command_script("Ship it on Friday.", rng_choosing("start over"), llm, seed=0)
    assert s is not None
    assert s.text == "The build is broken. We should roll back. Start over. Ship it on Friday."


def test_a_correction_always_ends_on_the_passage_words():
    passage = "Let's meet at 4 on Friday."
    llm = FakeLLM('{"scope": "word", "anchor": "4", "wrong": "3"}')
    s = correction_script(passage, rng_asking_for("word"), llm, seed=0)
    assert s is not None
    assert s.text.startswith("Let's meet at 3, ")
    assert s.text.endswith("4 on Friday.")
    assert s.tags["scope"] == "word"


def test_a_correction_whose_anchor_is_not_in_the_passage_is_rejected():
    llm = FakeLLM('{"scope": "word", "anchor": "five", "wrong": "3"}')
    assert correction_script("Let's meet at 4.", rng_asking_for("word"), llm, seed=0) is None


def test_a_sentence_correction_replaces_a_whole_sentence():
    passage = "The build is green. The demo is on Friday."
    llm = FakeLLM(
        '{"scope": "sentence", "anchor": "The demo is on Friday.", "wrong": "The demo is cancelled."}'
    )
    s = correction_script(passage, rng_asking_for("sentence"), llm, seed=0)
    assert s is not None
    assert s.text.startswith("The build is green. The demo is cancelled. ")
    assert s.text.endswith("The demo is on Friday.")
    assert s.tags["scope"] == "sentence"


def test_a_sentence_correction_on_part_of_a_sentence_is_rejected():
    llm = FakeLLM('{"scope": "sentence", "anchor": "on Friday.", "wrong": "on Monday."}')
    assert (
        correction_script("The demo is on Friday.", rng_asking_for("sentence"), llm, seed=0) is None
    )


def test_the_scope_asked_for_must_be_the_scope_returned():
    llm = FakeLLM('{"scope": "phrase", "anchor": "on Friday", "wrong": "on Monday"}')
    assert correction_script("The demo is on Friday.", rng_asking_for("word"), llm, seed=0) is None


def test_disfluencies_keep_every_passage_word_in_order():
    passage = "We need to ship the new build to the platform team before the review on Friday."
    llm = FakeLLM('{"anchor": "ship the new build", "abandoned": "send out the"}')
    s = disfluent_script(passage, random.Random(3), llm, seed=0)
    assert s is not None
    assert words_in_order(passage, s.text, min_ratio=1.0)
    assert s.tags["false_start"] == "1"
    assert words_in_order("send out the ship the new build", s.text, min_ratio=1.0)


def test_jobs_go_passage_by_passage_and_skip_what_is_done():
    from eval.finaltext.spoken import jobs_for

    jobs = jobs_for([("a", "A."), ("b", "B.")], ["natural", "explicit"], done={("a", "natural")})
    assert jobs == [("a", "A.", "explicit"), ("b", "B.", "natural"), ("b", "B.", "explicit")]
