import pytest

from utter_tools import schema


def row(pid: str, category: str, source: str = "synthetic", text: str = "Some text.") -> dict:
    return {
        "id": f"{pid}-{category}",
        "category": category,
        "domain": "everyday-text",
        "final_text": text,
        "dictionary": [],
        "lists": True,
        "source": source,
        "license": "cc-by-sa-4.0",
    }


def corpus(n: int) -> list[dict]:
    return [row(f"{i:012x}", c) for i in range(n) for c in ("natural", "disfluent")]


def test_passage_ids_from_every_kind_of_id():
    assert schema.passage_id("0123456789ab-correction") == "0123456789ab"
    assert schema.passage_id("kokoro-real/0123456789ab-correction") == "0123456789ab"


def test_a_passage_stays_in_one_split_and_test_gets_its_share():
    rows = corpus(1000)
    split = schema.split_by_passage(rows)
    assert set(split) == {schema.passage_id(r["id"]) for r in rows}
    test = sum(split[schema.passage_id(r["id"])] == "test" for r in rows) / len(rows)
    assert abs(test - schema.TEST_SHARE) < 0.01


def test_the_split_is_the_same_every_time():
    rows = corpus(300)
    assert schema.split_by_passage(rows) == schema.split_by_passage(list(reversed(rows)))


def test_published_passages_keep_their_split_and_new_ones_are_balanced():
    old = corpus(1000)
    before = schema.split_by_passage(old)
    new = [row(f"{i:012x}", "natural") for i in range(1000, 1500)]
    after = schema.split_by_passage(old + new, fixed=before)
    assert all(after[p] == s for p, s in before.items())
    fresh = [after[schema.passage_id(r["id"])] for r in new]
    assert abs(fresh.count("test") / len(fresh) - schema.TEST_SHARE) < 0.03


@pytest.mark.parametrize(
    ("raw", "fixed"),
    [
        ("Is it done.?", "Is it done?"),
        ("Yes ,no", "Yes ,no"),  # no space after: not the slip
        ("Fine , thanks.", "Fine, thanks."),
        ("Wait.. what", "Wait. what"),
        ("Wait... what", "Wait... what"),
        ("Use `a ,b` here.", "Use `a ,b` here."),  # code is left alone
    ],
)
def test_fix_final_fixes_only_mechanical_slips(raw: str, fixed: str):
    assert schema.fix_final(raw) == fixed


def test_licenses_map_to_the_datasets_ids():
    assert schema.license_id("https://creativecommons.org/licenses/by-sa/4.0/") == "cc-by-sa-4.0"
    assert schema.license_id("CC BY-SA 4.0") == "cc-by-sa-4.0"
    assert schema.license_id("generated") == schema.DATASET_LICENSE
    with pytest.raises(ValueError):
        schema.license_id("see repository")


def test_annotations_round_trip():
    tags = {"scope": "word", "cue": "no wait"}
    assert schema.from_annotations(schema.to_annotations(tags)) == tags
