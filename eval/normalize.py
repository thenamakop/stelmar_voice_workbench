"""Script-aware text normalisation for scoring English, Hindi, Urdu and mixed transcripts.

Every word is normalised by its own script, so a code-switched sentence gets English rules for its
Latin words, Hindi rules for its Devanagari words and Urdu rules for its Perso-Arabic words. The
same function is applied to references and hypotheses.

- Latin (English): lower-case; punctuation removed (apostrophes join, other marks split).
- Devanagari (Hindi): NFC; vowel signs (matras), virama, anusvara, chandrabindu and nukta are all
  kept; nukta letters are unified to base letter + U+093C whether written precomposed (क़, ऩ) or
  decomposed; danda and other punctuation removed; ZWJ and ZWNJ removed.

- Perso-Arabic (Urdu): NFC; Arabic yeh and alef maksura (ي ى) become Farsi yeh (ی), Arabic kaf (ك)
  becomes keheh (ک), Arabic heh (ه) becomes heh goal (ہ); ی + hamza above becomes ئ; optional
  diacritics (harakat, shadda, sukun, superscript alef, Quranic marks) and tatweel removed; heh
  doachashmee (ھ), hamza and madda are kept.
- Invisible format characters (Unicode category Cf: LRM, RLM, ALM, BOM, soft hyphen, ZWJ, ZWNJ, bidi
  controls) are removed everywhere before words are split, so they never count as words. A zero-width
  space (U+200B, also Cf) marks a word break, so it becomes a space instead.
- Digits in any script become ASCII digits; thousands separators inside numbers are dropped and a
  decimal point between digits is kept.

Nukta, two scores (decided 8 Nov 2026; PLAN section 6 says "nukta forms unified"):
- Primary (every choice rule in PLAN section 6 uses it): `normalize` keeps the nukta and only unifies
  the two Unicode encodings of the same letter, so क़ == क + ़ but ज़ != ज.
- Secondary, reported beside it and labelled "nukta ignored": `drop_nukta(normalize(text))` removes
  U+093C, so ज़ == ज. Transcribers use the nukta inconsistently; the tables flag any dataset where the
  two scores rank models differently.

Whisper's default BasicTextNormalizer is never used: it deletes combining marks, which strips Hindi
vowel signs and flatters Hindi WER (docs/PLAN.md section 6).
"""

from __future__ import annotations

import re
import unicodedata

LATIN = "latin"
DEVANAGARI = "devanagari"
ARABIC = "arabic"
DIGIT = "digit"
OTHER = "other"

# Nukta letters NFC keeps precomposed (U+0958..U+095F are composition exclusions, so NFC already
# decomposes those). Spelled out as code points so the mapping is visible in review.
_DEVANAGARI_NUKTA = {
    "ऩ": "ऩ",  # ऩ -> न + nukta
    "ऱ": "ऱ",  # ऱ -> र + nukta
    "ऴ": "ऴ",  # ऴ -> ळ + nukta
}
_DEVANAGARI_REMOVE = {"‌", "‍"}  # ZWNJ, ZWJ

_URDU_LETTERS = {
    "ي": "ی",  # ي Arabic yeh -> ی Farsi yeh
    "ى": "ی",  # ى alef maksura -> ی
    "ك": "ک",  # ك Arabic kaf -> ک keheh
    "ه": "ہ",  # ه Arabic heh -> ہ heh goal
}
# Optional diacritics: honorific signs, harakat (fathatan .. sukun), subscript alef and other
# marks U+0656..U+065F, superscript alef, Quranic annotation marks; plus tatweel and ZWNJ/ZWJ.
# Kept: U+0653 madda, U+0654 hamza above, U+0655 hamza below (they change the letter).
_URDU_REMOVE = re.compile(
    "[ؐ-ًؚ-ْٖ-ٰٟۖ-ۭـ‌‍]"
)

_DIGITS = {}
for _base in (0x0966, 0x0660, 0x06F0):  # Devanagari, Arabic-Indic, Extended Arabic-Indic
    for _i in range(10):
        _DIGITS[chr(_base + _i)] = str(_i)
_DIGIT_TABLE = str.maketrans(_DIGITS)

_THOUSANDS = re.compile(r"(?<=\d)[,٬](?=\d{3}(?:\D|$))")
_DECIMAL = re.compile(r"(?<=\d)[.٫](?=\d)")
_DECIMAL_MARK = ""  # private-use placeholder protecting decimal points from punctuation removal
_APOSTROPHES = "'’ʼ‘`"


def char_script(ch: str) -> str:
    cp = ord(ch)
    if ch.isdigit():
        return DIGIT
    if 0x0900 <= cp <= 0x097F or 0xA8E0 <= cp <= 0xA8FF:
        return DEVANAGARI
    if 0x0600 <= cp <= 0x06FF or 0x0750 <= cp <= 0x077F or 0xFB50 <= cp <= 0xFDFF or 0xFE70 <= cp <= 0xFEFF:
        return ARABIC
    if ch.isalpha() and unicodedata.name(ch, "").startswith("LATIN"):
        return LATIN
    return OTHER


def word_script(word: str) -> str:
    """Majority script of a word's letters (digits only count when there are no letters)."""
    counts: dict[str, int] = {}
    for ch in word:
        s = char_script(ch)
        if s != OTHER:
            counts[s] = counts.get(s, 0) + 1
    letters = {k: v for k, v in counts.items() if k != DIGIT}
    if letters:
        return max(letters, key=lambda k: letters[k])
    return DIGIT if counts else OTHER


def _split_scripts(word: str) -> list[str]:
    """Split a token where the script changes (Latin, Devanagari, Arabic); combining marks and
    digits stay with the run they follow."""
    parts: list[str] = []
    current = ""
    current_script = None
    for ch in word:
        s = char_script(ch)
        if s in (OTHER, DIGIT) or unicodedata.category(ch).startswith("M"):
            current += ch
            continue
        if current_script is not None and s != current_script:
            parts.append(current)
            current = ""
        current_script = s
        current += ch
    if current:
        parts.append(current)
    return parts


def _strip_punct(text: str) -> str:
    """Punctuation and symbols become spaces; combining marks (vowel signs, nukta) are kept."""
    out = []
    for ch in text:
        cat = unicodedata.category(ch)
        out.append(" " if cat[0] in "PSZ" or cat == "Cc" else ch)
    return "".join(out)


def _latin(word: str) -> str:
    return word.lower()


def _devanagari(word: str) -> str:
    word = unicodedata.normalize("NFC", word)
    for src, dst in _DEVANAGARI_NUKTA.items():
        word = word.replace(src, dst)
    return "".join(ch for ch in word if ch not in _DEVANAGARI_REMOVE)


def _arabic(word: str) -> str:
    word = unicodedata.normalize("NFC", word)
    for src, dst in _URDU_LETTERS.items():
        word = word.replace(src, dst)
    word = _URDU_REMOVE.sub("", word)
    word = word.replace("یٔ", "ئ")  # ی + hamza above -> ئ
    return unicodedata.normalize("NFC", word)


_BY_SCRIPT = {LATIN: _latin, DEVANAGARI: _devanagari, ARABIC: _arabic}


def normalize_word(word: str) -> str:
    fn = _BY_SCRIPT.get(word_script(word))
    return fn(word) if fn else word


def normalize(text: str) -> str:
    """Normalise a transcript for WER and CER. Language-independent: each word by its script."""
    text = unicodedata.normalize("NFC", text)
    text = text.replace("\u200b", " ")  # zero-width space: a word break
    text = "".join(ch for ch in text if unicodedata.category(ch) != "Cf").translate(_DIGIT_TABLE)
    text = _THOUSANDS.sub("", text)
    text = _DECIMAL.sub(_DECIMAL_MARK, text)
    for a in _APOSTROPHES:
        text = text.replace(a, "")
    # The placeholder is a private-use character (category Co), so punctuation removal keeps it.
    text = _strip_punct(text).replace(_DECIMAL_MARK, ".")
    words: list[str] = []
    for token in text.split():
        for part in _split_scripts(token):
            norm = normalize_word(part).strip()
            if norm:
                words.append(norm)
    return " ".join(words)


def drop_nukta(normalized: str) -> str:
    """Secondary Hindi score: remove the nukta (U+093C) from already-normalised text."""
    return normalized.replace("\u093c", "")


def scripts_in(text: str) -> set[str]:
    return {s for w in text.split() if (s := word_script(w)) in (LATIN, DEVANAGARI, ARABIC)}


def is_mixed(text: str) -> bool:
    """True when the text has words in more than one of Latin, Devanagari and Perso-Arabic."""
    return len(scripts_in(normalize(text))) > 1


EXPECTED_SCRIPT = {"en": LATIN, "hi": DEVANAGARI, "ur": ARABIC}
