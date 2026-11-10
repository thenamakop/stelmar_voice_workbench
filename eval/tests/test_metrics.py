"""WER, CER, Latin→Devanagari tracking, wrong-script rate and language detection."""

import pytest

import metrics


def scored(ref, hyp, lang="hi", mixed=False, uid="u"):
    return metrics.prepare(uid, ref, hyp, lang, mixed)


def test_wer_ignores_case_and_punctuation():
    assert metrics.wer([scored("Hello, world.", "hello world", "en")]) == 0.0


def test_wer_counts_hindi_vowel_sign_errors():
    # One wrong matra is one word error, not zero (the default Whisper normaliser would score 0).
    assert metrics.wer([scored("कल आना", "काल आना")]) == pytest.approx(0.5)


def test_cer_hindi():
    assert metrics.cer([scored("कल", "काल")]) == pytest.approx(0.5)  # one inserted vowel sign over 2 chars


def test_empty_references_are_excluded():
    items = [scored("।", "कुछ"), scored("नमस्ते", "नमस्ते")]
    assert metrics.wer(items) == 0.0
    assert metrics.wer([scored("", "x")]) is None


def test_empty_hypothesis_is_all_deletions():
    assert metrics.wer([scored("एक दो", "")]) == 1.0


def test_latin_words_in_devanagari():
    items = [
        scored("मैं office जा रहा हूँ", "मैं ऑफिस जा रहा हूँ", mixed=True, uid="a"),
        scored("meeting के बाद lunch", "meeting के बाद लंच", mixed=True, uid="b"),
        scored("project पूरा हुआ", "पूरा हुआ", mixed=True, uid="c"),
    ]
    lw = metrics.latin_words(items)
    assert lw.ref_latin_words == 4
    assert lw.in_devanagari == 2
    assert lw.kept_latin == 1
    assert lw.deleted == 1
    assert lw.devanagari_rate == pytest.approx(0.5)


def test_latin_words_none_when_reference_has_no_english():
    assert metrics.latin_words([scored("नमस्ते दुनिया", "नमस्ते दुनिया")]).devanagari_rate is None


def test_wrong_script_rate_catches_translation():
    items = [scored("मैं घर जा रहा हूँ", "he walks to the market", lang="hi")]
    assert metrics.wrong_script_rate(items) == 1.0
    assert metrics.wrong_script_rate([scored("یہ کتاب ہے", "یہ کتاب ہے", lang="ur")]) == 0.0
    # Mixed utterances are excluded: English words are expected there.
    assert metrics.wrong_script_rate([scored("मैं office", "मैं office", mixed=True)]) is None


def test_script_mix():
    mix = metrics.script_mix([scored("x", "मैं office जा", mixed=True)])
    assert mix == {"devanagari": pytest.approx(2 / 3, abs=1e-4), "latin": pytest.approx(1 / 3, abs=1e-4)}


def test_language_detection():
    d = metrics.LangDetection()
    d.add("hi", "hi", 4.0)
    d.add("hi", "en", 1.0)
    d.add("en", "en", 5.0)
    assert d.accuracy == pytest.approx(2 / 3)
    assert d.accuracy_by_time == pytest.approx(0.9)
    assert d.confusion[("hi", "en")] == 1
