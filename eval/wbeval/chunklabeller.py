"""Chunk labels derived from the REFERENCE text alone (Phase 5, item 4; owner's method of 24 Nov 2026).

The tuning clips have a reference text but no hand labels per chunk. A chunk's label is derived from the reference words
that fall inside its time range, where the words are placed by one rule that depends on no decoder, no language rule and no
model: the reference words of an utterance are spread over the utterance's duration in proportion to their characters (a
word of eight characters takes twice the time of a word of four). A chunk is then labelled by the script of the words that
fall in it, Latin against Devanagari (a word counts where its midpoint falls):

* `main`: `en` when the Latin words are at least half of the Latin and Devanagari words, else `hi`; an exact tie goes to
  the script of the first word of the chunk;
* `kind`: `pure` when the smaller of the two shares is under 15% (fixed in advance, 24 Nov 2026, not tuned), else `mixed`;
* a chunk with fewer than `MIN_WORDS` such words is not labelled (too few words to say).

Because it depends on nothing the engine decides, the label of a chunk holds for every detection rule; and a rule whose
decode is wrong (a Hindi chunk decoded as English text, which is what the labels are for) cannot change it.

Known limit: speech is not uniform; pauses and slow words move the real time of a word away from its place here. A boundary
error of two or three words changes the label only for chunks whose minor share is near 15% or whose two shares are near
equal. The method is validated on the ten hand-labelled held-out chunks (`validate_against_hand_labels`) and by the owner's
ear check on a random sample of the tuning chunks; it tunes nothing.
"""

from __future__ import annotations

import random
from dataclasses import dataclass

from normalize import DEVANAGARI, LATIN, word_script

PURE_BELOW = 0.15
MIN_WORDS = 4


@dataclass
class Word:
    text: str
    script: str
    mid_ms: float


def place_words(text: str, duration_ms: float) -> list[Word]:
    """The reference words of one utterance with the time of their midpoints; only Latin and Devanagari words are kept."""
    words = [w for w in text.split() if word_script(w) in (LATIN, DEVANAGARI)]
    total = sum(len(w) for w in words)
    if not words or total == 0:
        return []
    placed, cum = [], 0
    for w in words:
        placed.append(Word(w, word_script(w), (cum + len(w) / 2) / total * duration_ms))
        cum += len(w)
    return placed


def label_range(words: list[Word], start_ms: float, end_ms: float) -> dict | None:
    """{main, kind, latin, devanagari, latin_share} for the words that fall in [start_ms, end_ms), or None if too few."""
    inside = [w for w in words if start_ms <= w.mid_ms < end_ms]
    latin = sum(1 for w in inside if w.script == LATIN)
    dev = sum(1 for w in inside if w.script == DEVANAGARI)
    if latin + dev < MIN_WORDS:
        return None
    share = latin / (latin + dev)
    if latin == dev:
        main = "en" if inside[0].script == LATIN else "hi"
    else:
        main = "en" if share > 0.5 else "hi"
    kind = "pure" if min(share, 1 - share) < PURE_BELOW else "mixed"
    return {"main": main, "kind": kind, "latin": latin, "devanagari": dev, "latin_share": round(share, 3)}


def validate_against_hand_labels(hand: list, references: dict[str, tuple[str, float]]) -> dict:
    """Runs the labeller on the held-out chunks' time ranges and counts agreement with the owner's hand labels.
    `hand`: ChunkLabel list; `references`: utterance id -> (reference text, duration in seconds). Validates the labeller
    only: it changes no threshold and tunes no rule."""
    rows = []
    for lab in hand:
        text, dur = references[lab.id]
        got = label_range(place_words(text, dur * 1000.0), lab.start_ms, lab.end_ms)
        rows.append({"id": lab.id, "start_ms": lab.start_ms, "end_ms": lab.end_ms, "hand_main": lab.main,
                     "hand_kind": lab.kind, "derived_main": got and got["main"], "derived_kind": got and got["kind"],
                     "latin_share": got and got["latin_share"]})
    labelled = [r for r in rows if r["derived_main"]]
    return {"chunks": len(rows), "labelled": len(labelled),
            "main_agree": sum(1 for r in labelled if r["derived_main"] == r["hand_main"]),
            "kind_agree": sum(1 for r in labelled if r["derived_kind"] == r["hand_kind"]), "rows": rows}


def sample_for_ear_check(chunks: list[dict], n: int, seed: int, near: float = 0.10) -> list[dict]:
    """A random sample of `n` chunks for the owner's ear check: half led in English and half in Hindi where the labels
    allow, and at least three within `near` of the pure threshold (when that many exist)."""
    rng = random.Random(seed)
    close = [c for c in chunks if abs(min(c["latin_share"], 1 - c["latin_share"]) - PURE_BELOW) <= near]
    picked = rng.sample(close, min(3, len(close)))
    rest = [c for c in chunks if c not in picked]
    for main in ("en", "hi"):
        pool = [c for c in rest if c["main"] == main]
        want = n // 2 - sum(1 for c in picked if c["main"] == main)
        take = rng.sample(pool, max(0, min(want, len(pool))))
        picked += take
        rest = [c for c in rest if c not in take]
    if len(picked) < n:
        picked += rng.sample(rest, min(n - len(picked), len(rest)))
    return sorted(picked, key=lambda c: (c["id"], c["start_ms"]))
