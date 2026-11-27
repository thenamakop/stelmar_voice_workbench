"""Classifies the word errors of finished decodes, with no new decoding (Phase 5, item 4). Numbers only: no text.

    eval/.venv/Scripts/python -X utf8 -I eval/analyse_errors.py <run> [<run> ...] [--out docs/evidence/phase-5/<name>.txt]

Every error word of an utterance (a reference word substituted or deleted, a hypothesis word inserted; the same
alignment and normalisation as the WER) goes into exactly one kind, tested in this order:

* loop: the error word lies in a repetition loop of the hypothesis (a run of at least three repeats of a block of one to
  eight words; every word of the run beyond its first block);
* truncation: a reference word deleted in a run of more than 20 deleted words (the harness's own `truncated` limit);
* script: a substituted word whose reference is Latin (English) and whose hypothesis is Devanagari, or the other way;
* misrecognition: every other substitution, insertion or deletion.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

import jiwer

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402
from normalize import DEVANAGARI, LATIN, word_script  # noqa: E402

MAX_BLOCK = 8
MIN_REPEATS = 3
TRUNCATION_RUN = 20


def loop_words(hyp: list[str]) -> set[int]:
    """Indices of the hypothesis words that are in a repetition loop beyond the first block of the run."""
    inside: set[int] = set()
    i = 0
    while i < len(hyp):
        hit = False
        for p in range(1, MAX_BLOCK + 1):
            block = hyp[i:i + p]
            if len(block) < p:
                break
            n = 1
            while hyp[i + n * p:i + (n + 1) * p] == block:
                n += 1
            if n >= MIN_REPEATS:
                inside.update(range(i + p, i + n * p))
                i += n * p
                hit = True
                break
        if not hit:
            i += 1
    return inside


def classify(ref_text: str, hyp_text: str, lang: str = "hi", mixed: bool = True) -> Counter:
    s = metrics.prepare("x", ref_text, hyp_text, lang, mixed)
    ref, hyp = s.ref.split(), s.hyp.split()
    out: Counter = Counter()
    out["ref_words"] = len(ref)
    out["hyp_words"] = len(hyp)
    if not ref:
        return out
    if not hyp:
        out["truncation" if len(ref) > TRUNCATION_RUN else "misrecognition"] += len(ref)
        return out
    loops = loop_words(hyp)
    out["loops"] = sum(1 for k in sorted(loops) if k - 1 not in loops)
    for ch in jiwer.process_words(s.ref, s.hyp).alignments[0]:
        if ch.type == "equal":
            continue
        n_ref, n_hyp = ch.ref_end_idx - ch.ref_start_idx, ch.hyp_end_idx - ch.hyp_start_idx
        if ch.type == "delete":
            out["truncation" if n_ref > TRUNCATION_RUN else "misrecognition"] += n_ref
        elif ch.type == "insert":
            for k in range(ch.hyp_start_idx, ch.hyp_end_idx):
                out["loop" if k in loops else "misrecognition"] += 1
        else:  # substitute: the same number of words on both sides
            for r, h in zip(range(ch.ref_start_idx, ch.ref_end_idx), range(ch.hyp_start_idx, ch.hyp_end_idx)):
                if h in loops:
                    out["loop"] += 1
                elif {word_script(ref[r]), word_script(hyp[h])} == {LATIN, DEVANAGARI}:
                    out["script"] += 1
                else:
                    out["misrecognition"] += 1
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="+", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    kinds = ("loop", "truncation", "script", "misrecognition")
    L = ["Error kinds of finished decodes (Phase 5, item 4); numbers only; written by eval/analyse_errors.py.",
         "Kinds: loop = words in a repetition loop; truncation = reference words deleted in a run over 20; script = English word "
         "written in Devanagari or the reverse; misrecognition = every other error word.", "",
         "run | clip | ref words | hyp words | loop | truncation | script | misrecognition | all errors | loops (count)"]
    for run in a.runs:
        total: Counter = Counter()
        for line in (run / "utterances.jsonl").read_text(encoding="utf-8").splitlines():
            rec = json.loads(line)
            if "error" in rec:
                continue
            c = classify(rec["ref"], rec["hyp"], rec.get("lang", "hi"), rec.get("mixed", True))
            total.update(c)
            L.append(f"{run.name} | {rec['id']} | {c['ref_words']} | {c['hyp_words']} | " + " | ".join(str(c[k]) for k in kinds)
                     + f" | {sum(c[k] for k in kinds)} | {c['loops']}")
        L.append(f"{run.name} | all | {total['ref_words']} | {total['hyp_words']} | " + " | ".join(str(total[k]) for k in kinds)
                 + f" | {sum(total[k] for k in kinds)} | {total['loops']}")
        L.append("")
    text = "\n".join(L) + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
