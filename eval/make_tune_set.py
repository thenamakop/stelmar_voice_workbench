"""Builds the Phase 5 tuning set for the language-detection rules from CoSHE-Eval (evaluation only, never quoted).

    eval/.venv/Scripts/python -X utf8 -I eval/make_tune_set.py

Writes `eval/clips/phase5-tune-coshe.txt` (dataset<TAB>utterance id, like every clip list) and
`eval/labels/coshe-tune.json` (id, class, share of Devanagari letters; no text and no audio).

Where the labels come from: the class of a whole utterance is derived from its REFERENCE text by script, never by a
person and never by a model. `pure-en`: no Devanagari letter in the reference; `pure-hi`: no Latin letter; `mixed`:
both. This is a label of the utterance, not of a chunk (a chunk's language needs its own time range, which only the
ten hand-labelled held-out chunks have). So the set tunes:
  * chunk language on the pure utterances (every chunk of a pure utterance has that utterance's language);
  * transcript WER, script correctness, English words in Devanagari, translated chunks and the cost in decodes on all
    of them, which need no chunk labels.
Never in the set: the three clips that hold the ten held-out chunks (eval/clips/phase5-coshe-heldout.txt) and the
first 20 clips of the manifest (the 20 clips of the previous-text control runs), so the held-out final test and the
controls stay untouched. Seeded, so the same set comes out every time.
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path

EVAL = Path(__file__).resolve().parent
SEED = 20261124
SIZE = {"mixed": 40, "pure-en": 20, "pure-hi": 10}


def klass(text: str) -> tuple[str, float]:
    dev = sum(1 for c in text if "ऀ" <= c <= "ॿ" and c.isalpha())
    lat = sum(1 for c in text if c.isascii() and c.isalpha())
    total = dev + lat
    share = dev / total if total else 0.0
    if dev == 0:
        return "pure-en", 0.0
    if lat == 0:
        return "pure-hi", 1.0
    return "mixed", round(share, 3)


def main() -> int:
    manifest = [json.loads(line) for line in (EVAL / "data" / "coshe-eval" / "manifest.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
    held = {line.split("\t")[1].strip() for line in (EVAL / "clips" / "phase5-coshe-heldout.txt").read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.startswith("#")}
    controls = {r["id"] for r in manifest[:20]}
    pool: dict[str, list[dict]] = {k: [] for k in SIZE}
    for r in manifest:
        if r["id"] in held or r["id"] in controls:
            continue
        k, share = klass(r["text"])
        pool[k].append({"id": r["id"], "class": k, "devanagari_share": share, "duration_s": round(float(r["duration_s"]), 1)})
    rng = random.Random(SEED)
    chosen: list[dict] = []
    for k, n in SIZE.items():
        rows = sorted(pool[k], key=lambda x: x["id"])
        chosen += rng.sample(rows, min(n, len(rows)))
    chosen.sort(key=lambda x: x["id"])
    lines = ["# Phase 5 tuning set for the language-detection rules (eval/make_tune_set.py, seed %d)." % SEED,
             "# CoSHE-Eval clips other than the three held-out ones and the first 20 of the manifest; ids only: no audio, no text."]
    lines += [f"coshe-eval\t{x['id']}" for x in chosen]
    (EVAL / "clips" / "phase5-tune-coshe.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    labels = {
        "dataset": "coshe-eval",
        "about": "Utterance classes derived from the reference text by script (no text, no audio). Tuning only: the three held-out "
                 "clips and the first 20 manifest clips are excluded. See eval/make_tune_set.py.",
        "seed": SEED,
        "clips": chosen,
    }
    (EVAL / "labels" / "coshe-tune.json").write_text(json.dumps(labels, indent=1) + "\n", encoding="utf-8")
    by = {k: [x for x in chosen if x["class"] == k] for k in SIZE}
    for k, v in by.items():
        print(f"{k}: pool {len(pool[k])}, chosen {len(v)}, {sum(x['duration_s'] for x in v) / 60:.1f} min")
    print(f"total: {len(chosen)} clips, {sum(x['duration_s'] for x in chosen) / 60:.1f} min")
    return 0


if __name__ == "__main__":
    sys.exit(main())
