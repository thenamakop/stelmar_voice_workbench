"""Chunk language labels: scoring a run against hand labels, the held-out guard, translated chunks."""

import json

import pytest

from wbeval.labels import (ChunkLabel, HeldOutError, LABELS_DIR, is_translated, load_labels, score_chunks)


def seg(start, end, lang, text="x"):
    return {"start_ms": start, "end_ms": end, "lang": lang, "text": text}


def rec(uid, *segments):
    return {"id": uid, "segments": list(segments)}


def label(uid="a", start=1000, end=5000, kind="mixed", main="hi"):
    return ChunkLabel(uid, start, end, kind, main)


def test_a_chunk_is_credited_to_the_language_covering_most_of_it():
    r = rec("a", seg(1000, 2000, "en"), seg(2000, 5000, "hi"))
    out = score_chunks([r], [label(main="hi")])
    assert out["matches"] == 1 and out["rows"][0]["detected"] == "hi"
    out = score_chunks([r], [label(main="en")])
    assert out["matches"] == 0 and out["confusion"] == {"en->hi": 1}


def test_a_segment_outside_the_chunk_does_not_count():
    r = rec("a", seg(0, 900, "en"), seg(1000, 5000, "hi"), seg(5200, 9000, "en"))
    assert score_chunks([r], [label()])["rows"][0]["detected"] == "hi"


def test_nothing_decoded_and_missing_records_are_counted_not_scored_as_wrong():
    out = score_chunks([rec("a", seg(9000, 9500, "hi"))], [label(), label("b")])
    assert out["nothing_decoded"] == 1 and out["no_record"] == 1
    assert out["matches"] == 0 and out["accuracy"] == 0.0
    assert out["chunks"] == 2


def test_pure_and_mixed_are_reported_separately():
    r = rec("a", seg(1000, 5000, "hi"))
    out = score_chunks([r], [label(kind="pure"), label(kind="mixed", main="en")])
    assert out["pure"] == {"chunks": 1, "matches": 1}
    assert out["mixed"] == {"chunks": 1, "matches": 0}


def test_translated_means_a_hindi_led_chunk_with_only_latin_words():
    assert is_translated("hi", "the meeting is at three tomorrow")
    assert not is_translated("hi", "कल meeting है")
    assert not is_translated("hi", "ok yes")  # fewer than three Latin words
    assert not is_translated("en", "the meeting is at three tomorrow")
    assert is_translated("ur", "the meeting is at three tomorrow")
    assert not is_translated("ur", "ہماری meeting ہے")
    out = score_chunks([rec("a", seg(1000, 5000, "en", "we will talk about the budget"))], [label(main="hi")])
    assert out["translated"] == 1


def test_no_text_is_returned():
    out = score_chunks([rec("a", seg(1000, 5000, "hi", "secret words here"))], [label()])
    assert "secret" not in json.dumps(out)


def test_the_phase_2_labels_are_held_out_and_the_guard_refuses_tuning(tmp_path):
    assert len(load_labels("coshe-eval")) == 10
    assert len(load_labels("coshe-eval", heldout=True)) == 10
    with pytest.raises(HeldOutError):
        load_labels("coshe-eval", heldout=False)
    assert load_labels("no-such-set") == []
    assert (LABELS_DIR / "coshe-eval.json").exists()


def test_a_tuning_label_file_is_not_held_out(tmp_path):
    (tmp_path / "local-codeswitch.json").write_text(json.dumps({
        "dataset": "local-codeswitch", "heldout": False,
        "chunks": [{"id": "c1", "start_ms": 0, "end_ms": 4000, "kind": "mixed", "main": "en"}]}), encoding="utf-8")
    assert load_labels("local-codeswitch", heldout=False, labels_dir=tmp_path)[0].main == "en"
    assert load_labels("local-codeswitch", heldout=True, labels_dir=tmp_path) == []


def test_phase_2_label_file_matches_the_evidence_table():
    labels = load_labels("coshe-eval")
    assert [(c.id, c.start_ms, c.end_ms, c.kind, c.main) for c in labels][:3] == [
        ("audio_1343", 0, 12975, "mixed", "hi"), ("audio_350", 23770, 28415, "pure", "hi"),
        ("audio_147", 16830, 25250, "mixed", "en")]
    assert sum(1 for c in labels if c.kind == "pure") == 2 and sum(1 for c in labels if c.kind == "mixed") == 8
    assert sum(1 for c in labels if c.main == "en") == 5 and sum(1 for c in labels if c.main == "hi") == 5
