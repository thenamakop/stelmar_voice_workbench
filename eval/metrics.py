"""Scoring: WER, CER, English words written in Devanagari, output script mix and chunk language detection.

All text goes through normalize.normalize (script-aware) before scoring; references and hypotheses
are treated identically. Utterances whose normalised reference is empty are excluded from WER and
CER and counted separately.
"""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field

import jiwer

from normalize import ARABIC, DEVANAGARI, EXPECTED_SCRIPT, LATIN, drop_nukta, normalize, word_script


@dataclass
class Scored:
    id: str
    ref: str  # normalised
    hyp: str  # normalised
    lang: str
    mixed: bool


@dataclass
class LatinWords:
    """What happened to Latin-script (English) words of the reference."""

    ref_latin_words: int = 0
    kept_latin: int = 0  # aligned to a Latin hypothesis word (right or wrong spelling)
    in_devanagari: int = 0  # aligned to a Devanagari hypothesis word
    in_arabic: int = 0
    deleted: int = 0

    @property
    def devanagari_rate(self) -> float | None:
        return self.in_devanagari / self.ref_latin_words if self.ref_latin_words else None


@dataclass
class LangDetection:
    """Per-chunk language detection against the utterance's expected language."""

    chunks: int = 0
    correct: int = 0
    seconds: float = 0.0
    correct_seconds: float = 0.0
    confusion: Counter = field(default_factory=Counter)  # (expected, detected) -> chunks

    def add(self, expected: str, detected: str, seconds: float) -> None:
        self.chunks += 1
        self.seconds += seconds
        self.confusion[(expected, detected)] += 1
        if detected == expected:
            self.correct += 1
            self.correct_seconds += seconds

    @property
    def accuracy(self) -> float | None:
        return self.correct / self.chunks if self.chunks else None

    @property
    def accuracy_by_time(self) -> float | None:
        return self.correct_seconds / self.seconds if self.seconds else None


def prepare(uid: str, ref: str, hyp: str, lang: str, mixed: bool) -> Scored:
    return Scored(uid, normalize(ref), normalize(hyp), lang, mixed)


def nukta_ignored(items: list[Scored]) -> list[Scored]:
    """The same utterances with U+093C removed from reference and hypothesis (secondary Hindi score)."""
    return [Scored(s.id, drop_nukta(s.ref), drop_nukta(s.hyp), s.lang, s.mixed) for s in items]


def _scorable(items: list[Scored]) -> list[Scored]:
    return [s for s in items if s.ref]


def wer(items: list[Scored]) -> float | None:
    items = _scorable(items)
    if not items:
        return None
    return jiwer.wer([s.ref for s in items], [s.hyp for s in items])


def cer(items: list[Scored]) -> float | None:
    items = _scorable(items)
    if not items:
        return None
    # CER over characters of the normalised text, spaces included (jiwer's definition).
    return jiwer.cer([s.ref for s in items], [s.hyp for s in items])


def utterance_wer(s: Scored) -> float | None:
    return jiwer.wer(s.ref, s.hyp) if s.ref else None


def latin_words(items: list[Scored]) -> LatinWords:
    """Align each reference to its hypothesis and follow the reference's Latin-script words."""
    out = LatinWords()
    for s in _scorable(items):
        ref_words = s.ref.split()
        if not any(word_script(w) == LATIN for w in ref_words):
            continue
        hyp_words = s.hyp.split()
        if not hyp_words:
            n = sum(1 for w in ref_words if word_script(w) == LATIN)
            out.ref_latin_words += n
            out.deleted += n
            continue
        alignment = jiwer.process_words(s.ref, s.hyp).alignments[0]
        for chunk in alignment:
            for k in range(chunk.ref_start_idx, chunk.ref_end_idx):
                if word_script(ref_words[k]) != LATIN:
                    continue
                out.ref_latin_words += 1
                if chunk.type == "delete":
                    out.deleted += 1
                    continue
                # equal and substitute chunks pair words one to one
                h = chunk.hyp_start_idx + (k - chunk.ref_start_idx)
                script = word_script(hyp_words[h]) if h < chunk.hyp_end_idx else None
                if script == DEVANAGARI:
                    out.in_devanagari += 1
                elif script == ARABIC:
                    out.in_arabic += 1
                else:
                    out.kept_latin += 1
    return out


def script_mix(items: list[Scored]) -> dict[str, float]:
    """Share of hypothesis words per script."""
    counts: Counter = Counter()
    for s in items:
        counts.update(word_script(w) for w in s.hyp.split())
    total = sum(counts.values())
    return {k: round(v / total, 4) for k, v in sorted(counts.items())} if total else {}


def wrong_script_rate(items: list[Scored]) -> float | None:
    """Share of hypothesis words not in the expected script, for single-language utterances only
    (catches Hindi or Urdu speech coming back as English translation or in the other script)."""
    total = wrong = 0
    for s in items:
        if s.mixed or s.lang not in EXPECTED_SCRIPT:
            continue
        for w in s.hyp.split():
            script = word_script(w)
            if script in (LATIN, DEVANAGARI, ARABIC):
                total += 1
                wrong += script != EXPECTED_SCRIPT[s.lang]
    return wrong / total if total else None


def truncated(s: Scored, min_ratio: float = 0.7, max_deleted_run: int = 20) -> bool:
    """A hypothesis that lost a stretch of speech: fewer than min_ratio hypothesis words per reference
    word, or more than max_deleted_run reference words deleted in a row."""
    ref, hyp = s.ref.split(), s.hyp.split()
    if not ref:
        return False
    if len(hyp) / len(ref) < min_ratio:
        return True
    if not hyp:
        return len(ref) > max_deleted_run
    for chunk in jiwer.process_words(s.ref, s.hyp).alignments[0]:
        if chunk.type == "delete" and chunk.ref_end_idx - chunk.ref_start_idx > max_deleted_run:
            return True
    return False
