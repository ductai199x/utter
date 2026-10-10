from eval.finaltext.lists import ListPassage, lists_in, parse, prose_gold, script

PACK = ListPassage("Things to pack:", ["passport", "charger", "two shirts", "sunscreen"], False)
STEPS = ListPassage("To fix it:", ["Open the settings", "Turn off sync", "Restart the app"], True)


def test_markdown_and_parse_round_trip():
    assert PACK.markdown() == "Things to pack:\n- passport\n- charger\n- two shirts\n- sunscreen"
    assert STEPS.markdown().splitlines()[1:] == [
        "1. Open the settings",
        "2. Turn off sync",
        "3. Restart the app",
    ]
    for lp in (PACK, STEPS):
        back = parse(lp.markdown())
        assert back is not None
        assert (back.intro, back.items, back.ordered) == (lp.intro, lp.items, lp.ordered)


def test_spoken_scripts_enumerate_the_ways_people_do():
    assert script(PACK, "plain") == "Things to pack: passport, charger, two shirts and sunscreen"
    assert script(PACK, "bullets").startswith(
        "Things to pack: Bullet point, passport. Bullet point,"
    )
    assert script(STEPS, "ordinals") == (
        "To fix it: First, Open the settings. Second, Turn off sync. Third, Restart the app."
    )
    assert script(STEPS, "then") == (
        "To fix it: First, Open the settings. Then Turn off sync. And finally, Restart the app."
    )


def test_with_lists_off_the_gold_is_prose():
    assert (
        prose_gold(PACK, "plain") == "Things to pack: passport, charger, two shirts and sunscreen."
    )
    assert prose_gold(STEPS, "steps") == (
        "To fix it: Step 1: Open the settings. Step 2: Turn off sync. Step 3: Restart the app."
    )


def test_lists_are_found_with_their_intro_and_code_is_not():
    doc = (
        "Some text.\nYou need these tools:\n- a screwdriver\n- pliers\n- a multimeter\nMore text.\n"
        "Run:\n1. `sudo apt update`\n2. `sudo apt upgrade`\n3. reboot\n"
    )
    found = lists_in(doc)
    assert len(found) == 1
    assert found[0].intro == "You need these tools:"
    assert found[0].items == ["a screwdriver", "pliers", "a multimeter"]


def test_a_paragraph_is_not_a_list():
    assert parse("Just a sentence. And another.") is None
