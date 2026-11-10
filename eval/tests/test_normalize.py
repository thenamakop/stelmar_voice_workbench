"""Script-aware normaliser: English, Hindi (vowel signs kept, nukta unified), Urdu, mixed text."""

import unicodedata

import pytest

from normalize import ARABIC, DEVANAGARI, LATIN, is_mixed, normalize, word_script

# ------------------------------------------------------------------------------------------- English


def test_english_lower_case_without_punctuation():
    assert normalize("Hello, World! How are YOU?") == "hello world how are you"


def test_english_apostrophes_join_and_hyphens_split():
    assert normalize("It's a well-known fact.") == "its a well known fact"
    assert normalize("It’s") == normalize("It's") == "its"


def test_numbers_keep_decimals_and_drop_thousands_separators():
    assert normalize("It costs 1,000 rupees, or 3.5 dollars.") == "it costs 1000 rupees or 3.5 dollars"


# --------------------------------------------------------------------------------------------- Hindi

HINDI = "किताबें मेज़ पर रखी हैं।"


def test_vowel_signs_survive():
    out = normalize(HINDI)
    for sign in ("ि", "ा", "े", "ै", "ी", "ं"):  # ि ा े ै ी ं
        assert sign in out, f"vowel sign or anusvara U+{ord(sign):04X} was stripped"
    # Words that differ only by a vowel sign must stay different (Whisper's default normaliser
    # deletes combining marks and would make these equal).
    assert normalize("कल") != normalize("काल")
    assert normalize("दिन") != normalize("दीन")
    assert normalize("मिल") != normalize("मल")


def test_whisper_basic_normaliser_would_have_stripped_them():
    # Documents why we never use it: removing category M characters destroys Hindi words.
    stripped = "".join(c for c in unicodedata.normalize("NFKD", HINDI) if not unicodedata.category(c).startswith("M"))
    assert stripped.split()[0] != normalize(HINDI).split()[0]


def test_danda_and_punctuation_removed():
    assert normalize("यह अच्छा है। क्या आप आएँगे?") == normalize("यह अच्छा है क्या आप आएँगे")
    assert "।" not in normalize("यह अच्छा है।")
    assert normalize("पहला॥ दूसरा") == normalize("पहला दूसरा")


@pytest.mark.parametrize(
    "precomposed, decomposed",
    [
        ("ज़", "ज़"),  # ज़
        ("क़", "क़"),  # क़
        ("फ़", "फ़"),  # फ़
        ("ड़", "ड़"),  # ड़
        ("ऩ", "ऩ"),  # ऩ (NFC keeps this one precomposed)
        ("ऱ", "ऱ"),  # ऱ
        ("ऴ", "ऴ"),  # ऴ
    ],
)
def test_nukta_forms_unified(precomposed, decomposed):
    assert normalize(f"{precomposed}रूर") == normalize(f"{decomposed}रूर")
    assert "़" in normalize(f"{precomposed}रूर"), "nukta itself is kept"


def test_nukta_is_not_dropped():
    assert normalize("ज़रूर") != normalize("जरूर")


def test_zero_width_joiners_removed():
    assert normalize("क्‍ष") == normalize("क्ष")
    assert normalize("क्‌ष") == normalize("क्ष")


def test_chandrabindu_and_anusvara_kept_distinct():
    assert normalize("हूँ") != normalize("हूं")


def test_devanagari_digits_become_ascii():
    assert normalize("साल २०२६ में") == "साल 2026 में"


# ---------------------------------------------------------------------------------------------- Urdu


def test_urdu_letter_variants_unified():
    assert normalize("كتاب") == normalize("کتاب")  # Arabic kaf vs keheh
    assert normalize("علي") == normalize("علی")  # Arabic yeh vs Farsi yeh
    assert normalize("موسى") == normalize("موسی")  # alef maksura
    assert normalize("كه") == normalize("کہ")  # kaf and heh together


def test_urdu_optional_diacritics_removed():
    assert normalize("کِتاب") == normalize("کتاب")  # zer
    assert normalize("مُحَمَّد") == normalize("محمد")  # pesh, zabar, shadda
    assert normalize("اللّٰہ") == normalize("اللہ")  # shadda, superscript alef
    assert normalize("کـتاب") == normalize("کتاب")  # tatweel


def test_urdu_letters_that_change_meaning_are_kept():
    assert normalize("بھائی") == "بھائی"  # heh doachashmee (aspiration) is not heh goal
    assert normalize("بھائی") != normalize("بہائی")
    assert normalize("آج") != normalize("اج")  # madda
    assert normalize("یٔ") == normalize("ئ")  # ی + hamza above == ئ


def test_urdu_punctuation_and_digits():
    assert normalize("یہ اچھا ہے۔ کیا، آپ؟") == normalize("یہ اچھا ہے کیا آپ")
    assert normalize("سال ۲۰۲۶") == normalize("سال 2026") == "سال 2026"
    assert normalize("٣") == "3"


def test_urdu_stays_in_nfc():
    out = normalize("ۂ")
    assert out == unicodedata.normalize("NFC", out)


# --------------------------------------------------------------------------------------------- mixed


def test_mixed_hindi_english_each_word_by_its_script():
    out = normalize("मैं Office जा रहा हूँ, OK?")
    assert out == "मैं office जा रहा हूँ ok"
    assert [word_script(w) for w in out.split()] == [DEVANAGARI, LATIN, DEVANAGARI, DEVANAGARI, DEVANAGARI, LATIN]


def test_mixed_sentence_keeps_vowel_signs_and_lower_cases_english():
    out = normalize("आज की Meeting में हमने Machine Learning पर बात की।")
    assert out == "आज की meeting में हमने machine learning पर बात की"
    assert "ी" in out  # ी survives next to English words


def test_script_change_inside_a_token_splits_it():
    assert normalize("AIके बारे में") == "ai के बारे में"


def test_english_in_devanagari_is_not_equal_to_latin():
    # The harness reports these separately (Latin→Devanagari rate); normalisation must not hide them.
    assert normalize("मैं ऑफिस जा रहा हूँ") != normalize("मैं office जा रहा हूँ")


def test_mixed_urdu_english():
    assert normalize("یہ Project بہت اچھا ہے") == "یہ project بہت اچھا ہے"


def test_is_mixed():
    assert is_mixed("मैं office जा रहा हूँ")
    assert not is_mixed("मैं दफ़्तर जा रहा हूँ")
    assert not is_mixed("plain English text, 2026.")
    assert is_mixed("یہ Project ہے")


def test_word_script():
    assert word_script("नमस्ते") == DEVANAGARI
    assert word_script("سلام") == ARABIC
    assert word_script("hello") == LATIN


# ------------------------------------------------------------------------------------------- general


@pytest.mark.parametrize(
    "text",
    [HINDI, "Hello, World!", "مُحَمَّد كتاب", "मैं Office जा रहा हूँ, OK?", "ज़रूर ऩ", "1,000.5 ۲۰۲۶"],
)
def test_idempotent(text):
    once = normalize(text)
    assert normalize(once) == once


def test_whitespace_and_empty():
    assert normalize("  \t\n ") == ""
    assert normalize("।?!") == ""
    assert normalize("a b") == "a b"


@pytest.mark.parametrize("mark", ["\u200e", "\u200f", "\u061c", "\ufeff", "\u00ad", "\u2066", "\u2069"])
def test_invisible_format_characters_never_count_as_words(mark):
    # LRM, RLM, ALM, BOM, soft hyphen and bidi isolates (FLEURS-ur references contain LRM).
    assert normalize(f"یہ {mark} کتاب") == normalize("یہ کتاب")
    assert normalize(f"{mark}नमस्ते") == "नमस्ते"
    assert normalize(f"hello{mark}world") == "helloworld"


def test_zero_width_space_is_a_word_break():
    assert normalize("hello\u200bworld") == "hello world"
    assert normalize("یہ\u200bکتاب") == normalize("یہ کتاب")
