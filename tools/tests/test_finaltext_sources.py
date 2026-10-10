import pytest

from eval.finaltext.sources import dictatable, english, paragraphs, split_of


@pytest.mark.parametrize(
    "text",
    [
        "In the File, Options area, there's a General tab, and at the bottom of that is this setting.",
        "Hey Sam, can you send me the Q3 numbers by Friday? I need them for the board deck.",
        "It runs at ~240 videos/min (4× faster) on the new cluster, which is great news.",
        "Wait... are you sure about that? It seems a little too good to be true.",
        "Use a lighter config, e.g. the default one, and restart the service afterwards.",
    ],
)
def test_well_written_dictation_sized_text_is_kept(text: str):
    assert dictatable(text)


@pytest.mark.parametrize(
    "text",
    [
        "Yes , I am trying to fix errors using shellcheck before I run anything at all.",
        "I didn't know about this site before.. it's helpful to me as a beginner here.",
        "The actual case is a management system for a corporation. in the real application they differ.",
        "any clues to solve this problem with the login screen on my laptop ?",
        "i think the drivers are broken again after the latest kernel update today.",
        "The results are y_CP=(1.11 +- 0.22)% and the asymmetry is consistent with zero.",
        "See https://example.com/docs for the details on how to configure the new server.",
        "Call foo() with the right arguments and it should return the value you expect.",
        "You can install it manually: root@Kali:~# apt install whois Reading package lists... Done.",
        "Msg 927, Level 14, State 2, Line 2 Database cannot be opened while it is being restored.",
        "Too short.",
        "Biography Vladimirov was born in Yaroslavl",  # a heading glued to a sentence fragment
    ],
)
def test_sloppy_code_like_or_odd_text_is_dropped(text: str):
    assert not dictatable(text)


def test_paragraphs_with_indented_lines_are_dropped_whole():
    text = (
        "A normal paragraph with words.\n\n    indented code line\nmore text here.\n\nAnother one."
    )
    assert list(paragraphs(text)) == ["A normal paragraph with words.", "Another one."]


@pytest.mark.parametrize(
    "text",
    [
        "Kannst das USB3 nicht einfach mal im BIOS abschalten?",
        "Non ho ancora guardato ma è difficile come cosa?",
        "Ik ben zo blij met deze IRC. Dankzij dit clubje ben ik verslaafd aan Ubuntu.",
        "Les vélos sont acceptés à bord de certains cars équipés de rack à vélo.",
    ],
)
def test_other_languages_are_not_english(text: str):
    assert not english(text)


@pytest.mark.parametrize(
    "text",
    [
        "Homeschooling usually yields better results than many public schools.",
        "The die was cast, so we went on with the plan anyway.",
        "Conversion calculator converts units from meter per second squared.",
    ],
)
def test_english_without_many_common_words_is_english(text: str):
    assert english(text)


def test_the_split_is_stable_per_document():
    assert split_of("doc-1") == split_of("doc-1")
    splits = {split_of(f"doc-{i}") for i in range(200)}
    assert splits == {"eval", "train"}
