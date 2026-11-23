"""The Panel cause split (eval/panel_causes.py) and the night-speed reader (scripts/gates/night_speed.py)."""

import sys
from pathlib import Path

import panel_causes as pc

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "scripts" / "gates"))
import night_speed as ns  # noqa: E402


def test_skeleton_gives_the_same_word_in_devanagari_and_urdu_script():
    # "paani" (water), "kitaab" (book), "baat" (talk): the vowels differ in what each script writes, the consonants do not.
    assert pc.skeleton("पानी") == pc.skeleton("پانی") == "pn"
    assert pc.skeleton("किताब") == pc.skeleton("کتاب") == "ktb"
    assert pc.skeleton("बात") == pc.skeleton("بات") == "bt"


def test_the_flap_with_a_nukta_matches_the_urdu_flap():
    assert pc.skeleton("पढ़ा") == pc.skeleton("پڑا") == "pr"


def test_a_word_without_consonants_is_a_placeholder_and_latin_has_no_skeleton():
    assert pc.skeleton_text("और अब") == "r b"
    assert pc.skeleton("hello") == ""
    assert pc.skeleton_text("अ") == "_"


def test_the_script_mismatch_is_reported_when_no_hypothesis_word_is_in_the_reference_script():
    rows = [({"dataset": "hi-set", "lang": "hi", "text": "यह बात है"},
             [{"text": "یہ بات ہے", "lang": "ur", "start_ms": 0, "end_ms": 1000}] * 5)
            for _ in range(5)]
    out: list[str] = []
    pc.report(rows, {"hi": "ur", "ur": "ur", "en": "en"}, out.append)
    text = "\n".join(out)
    assert "SCRIPT MISMATCH" in text
    assert "hi-set: 5 utterances" in text
    assert "hi-set: skeleton WER" in text


def test_a_run_without_a_night_plan_or_night_units_is_reported_as_such(tmp_path):
    run = tmp_path / "r"
    run.mkdir()
    (run / "evidence.txt").write_text("nothing here\n", encoding="utf-8")
    out: list[str] = []
    ns.report(run, out.append)
    assert "no night plan" in "\n".join(out)


def test_the_night_speed_compares_the_plan_with_the_night_units(tmp_path):
    run = tmp_path / "r"
    run.mkdir()
    (run / "evidence.txt").write_text(
        "  18:03:00 night pass planned for 2 session(s): about 5.6 h, ready by 23:40 IST (measured speed: 2.44 times the "
        "profile's, from 5 finished unit(s))\n"
        "  night units finished: 19; audio 96.1 of 180.9 min; engine time 109.9 min; sessions\n"
        "  Panel unit 0 of post-1: 300 s of audio in 360 s on gpu -> 1.20, 0 restart(s)\n"
        "  Panel unit 0 of post-2: 300 s of audio in 330 s on gpu -> 1.10, 0 restart(s)\n", encoding="utf-8")
    out: list[str] = []
    ns.report(run, out.append)
    text = "\n".join(out)
    assert "1.20 engine seconds per audio second" in text and "1.10 engine seconds per audio second" in text
    assert "actual over planned 0.59" in text  # 1.10 / (5.6 h * 60 / 180.9 min)
