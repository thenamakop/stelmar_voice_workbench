"""Reference-only chunk labels (eval/wbeval/chunklabeller.py): they depend on the reference text and the time ranges only."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from wbeval import chunklabeller as cl  # noqa: E402

HI = "पुस्तकालय में नई किताबें रखी गईं"  # 6 Devanagari words
EN = "today we talk about battery storage"  # 6 Latin words


def test_words_are_spread_over_the_duration_by_their_characters():
    words = cl.place_words("ab cdef", 6000)  # 2 and 4 characters: the second word takes twice the time
    assert [round(w.mid_ms) for w in words] == [1000, 4000]


def test_a_chunk_is_labelled_by_the_script_of_the_words_that_fall_in_it():
    words = cl.place_words(HI + " " + EN, 10000)
    first = cl.label_range(words, 0, 5000)
    last = cl.label_range(words, 5000, 10000)
    assert first["main"] == "hi" and first["kind"] == "pure"
    assert last["main"] == "en" and last["kind"] == "pure"
    both = cl.label_range(words, 0, 10000)
    assert both["kind"] == "mixed" and both["latin"] == 6 and both["devanagari"] == 6


def test_pure_means_the_smaller_share_is_under_fifteen_percent():
    # one Hindi word of ten is 10%: pure; two of ten is 20%: mixed
    ten = cl.label_range(cl.place_words(" ".join(["word"] * 9) + " शब्द", 10000), 0, 10000)
    assert (ten["main"], ten["kind"]) == ("en", "pure")
    two = cl.label_range(cl.place_words(" ".join(["word"] * 8) + " शब्द शब्द", 10000), 0, 10000)
    assert (two["main"], two["kind"]) == ("en", "mixed")


def test_too_few_words_gives_no_label():
    assert cl.label_range(cl.place_words("one two three", 3000), 0, 3000) is None


def test_the_label_does_not_depend_on_anything_but_the_reference_and_the_range():
    # the same reference and range give the same label whatever a decoder produced: there is no decoder input at all
    a = cl.label_range(cl.place_words(HI + " " + EN, 9000), 2000, 8000)
    b = cl.label_range(cl.place_words(HI + " " + EN, 9000), 2000, 8000)
    assert a == b


def test_the_ear_check_sample_is_random_but_repeatable_and_stratified():
    chunks = []
    for i in range(80):
        main = "en" if i % 2 else "hi"
        kind = "mixed" if i % 4 < 2 else "pure"
        share = (0.7 if main == "en" else 0.3) if kind == "mixed" else (0.95 if main == "en" else 0.05)
        chunks.append({"id": f"c{i}", "start_ms": i, "end_ms": i + 5000, "main": main, "kind": kind, "latin_share": share})
    one = cl.sample_for_ear_check(chunks, 10, seed=1)
    assert one == cl.sample_for_ear_check(chunks, 10, seed=1)
    assert len(one) == 10
    for main in ("en", "hi"):
        for kind in ("mixed", "pure"):
            assert sum(1 for c in one if c["main"] == main and c["kind"] == kind) >= 2
