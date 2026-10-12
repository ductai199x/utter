import pytest

from utter_tools.text.numbers import spoken_digits


@pytest.mark.parametrize(
    ("spoken", "written"),
    [
        ("seventy two k", "72k"),
        ("one hundred eight k", "108k"),
        ("about two hundred forty videos", "~240 videos"),
        ("around two forty", "~240"),
        ("four times faster", "4× faster"),
        ("seven point five hours", "7.5 hours"),
        ("seven and a half hours", "7.5 hours"),
        ("t f thirty two", "TF32"),
        ("version two point one point three", "v2.1.3"),
        ("three fifteen p m", "3:15 PM"),
        ("twenty twenty six", "2026"),
        ("one thousand", "1,000"),
        ("a hundred people", "100 people"),
        ("one hundred and five", "105"),
        ("the third time", "the 3rd time"),
        ("twenty five dollars", "$25"),
        ("q three", "Q3"),
        ("nineteen", "19"),
        ("two four six", "2, 4, 6"),
        ("six twenty five a m", "06:25"),
        ("sixteen point zero four", "16.04"),
        ("zero point five", "0.5"),
        ("in the nineteen thirties", "in the 1930s"),
        ("the nineties", "the '90s"),
        ("nineteen oh five", "1905"),
        ("room four oh one", "room 401"),
        ("kubuntu sixteen point ten", "Kubuntu 16.10"),
        ("sixteen point oh four", "16.04"),
        ("zero point twenty five", "0.25"),
        ("twenty point five", "20.5"),
        ("nine a m on friday", "9:00 AM on Friday"),
        ("at two thirty", "at 2:30"),
        ("emperor theodosius the second", "Emperor Theodosius II"),
        ("world war two", "World War II"),
        ("louis the fourteenth", "Louis XIV"),
    ],
)
def test_spoken_and_written_numbers_give_the_same_digits(spoken: str, written: str):
    assert spoken_digits(spoken) == spoken_digits(written)


def test_a_misread_number_differs():
    assert spoken_digits("one hundred eighty eight k") != spoken_digits("108k")


def test_text_without_numbers_has_no_digits():
    assert spoken_digits("send it to the platform team") == ""


def test_oh_outside_a_number_is_not_zero():
    assert spoken_digits("oh, five more") == spoken_digits("oh, 5 more") == "5"
