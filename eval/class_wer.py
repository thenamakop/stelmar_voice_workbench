"""WER per chunk class of finished decodes, with no new decoding (Phase 5, item 4; owner's step 3a of 26 Nov 2026). Numbers only.

    eval/.venv/Scripts/python -X utf8 -I eval/class_wer.py --tune <run> [<run> ...] --heldout <run> [<run> ...] [--out docs/evidence/phase-5/class-wer.txt]

A chunk is a time range with a class (main language, pure or mixed): the 339 tuning chunks of eval/labels/coshe-tune-chunks.json
(derived from the reference text, validated by the owner's ear check) and the ten held-out hand-labelled chunks. For each chunk:

* each whole utterance is aligned once, reference against the words of all the run's segments, with the harness normalisation
  (`metrics.prepare`) and the word alignment of the WER (jiwer);
* a hypothesis word belongs to the chunk its segment's midpoint falls in; a matched or substituted reference word, and its error,
  count in the chunk of the hypothesis word it is aligned with; an inserted word is an error of its own chunk; a deleted
  reference word counts, with its error, in the chunk of the next hypothesis word (the previous one at the end).

The WER of a class is the sum of the errors over the sum of the reference words of its chunks. Reference words that fall in a
gap (a chunk the labeller skipped as too short or too few words, or a stretch with no label) are left out, so the class figures
sum to a little less than the corpus WER. The held-out chunks are ten ranges of three clips, so only their classes of one to
four chunks are small samples. The second table takes the pure English chunks a run detected as English (those a pair decodes
with the small model) and gives every run's WER on exactly those chunks, so the turbo model and the small model are compared on
the same audio. A held-out run only reads existing results; nothing is tuned on it.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

import jiwer

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402
from wbeval import data  # noqa: E402
from wbeval.labels import load_labels  # noqa: E402

EVAL = Path(__file__).resolve().parent
CLASSES = [("en", "pure"), ("hi", "pure"), ("en", "mixed"), ("hi", "mixed")]
NAMES = {("en", "pure"): "pure English", ("hi", "pure"): "pure Hindi", ("en", "mixed"): "English-led mixed",
         ("hi", "mixed"): "Hindi-led mixed"}


def score_run(run: Path, chunks: list[dict], refs: dict[str, tuple[str, float]]):
    """Per chunk: reference words and errors, from one word alignment of each whole utterance.

    Each hypothesis word belongs to the chunk its segment's midpoint falls in. An aligned pair (equal or substituted) counts
    its reference word and its error, if any, in the chunk of its hypothesis word; an inserted word is an error of its own chunk;
    a deleted reference word counts, with its error, in the chunk of the next hypothesis word (the previous one at the end)."""
    recs = {}
    for line in (run / "utterances.jsonl").read_text(encoding="utf-8").splitlines():
        r = json.loads(line)
        recs[r["id"]] = r
    by_utt: dict[str, list[dict]] = defaultdict(list)
    for c in chunks:
        by_utt[c["id"]].append(c)
    tot: dict[tuple, dict] = {}
    for uid, rec in recs.items():
        cs = by_utt.get(uid)
        if not cs:
            continue
        ref = metrics.prepare(uid, refs[uid][0], "", "hi", True).ref.split()
        hyp: list[str] = []
        owner: list[int | None] = []
        for g in rec.get("segments", []):
            mid = (g["start_ms"] + g["end_ms"]) / 2
            k = next((i for i, c in enumerate(cs) if c["start_ms"] <= mid < c["end_ms"]), None)
            words = metrics.prepare(uid, "", g["text"], "hi", True).hyp.split()
            hyp += words
            owner += [k] * len(words)
        if not ref or not hyp:
            continue
        for ch in jiwer.process_words(" ".join(ref), " ".join(hyp)).alignments[0]:
            def bump(k, n, e):
                if k is None:
                    return
                c = cs[k]
                t = tot.setdefault((c["id"], c["start_ms"], c["end_ms"]), {"cls": (c["main"], c["kind"]), "n": 0, "e": 0, "lang": defaultdict(int)})
                t["n"] += n
                t["e"] += e
            if ch.type == "insert":
                for h in range(ch.hyp_start_idx, ch.hyp_end_idx):
                    bump(owner[h], 0, 1)
            elif ch.type == "delete":
                h = min(ch.hyp_start_idx, len(hyp) - 1)
                for _ in range(ch.ref_start_idx, ch.ref_end_idx):
                    bump(owner[h], 1, 1)
            else:
                err = 0 if ch.type == "equal" else 1
                for h in range(ch.hyp_start_idx, ch.hyp_end_idx):
                    bump(owner[h], 1, err)
        for g in rec.get("segments", []):
            mid = (g["start_ms"] + g["end_ms"]) / 2
            k = next((i for i, c in enumerate(cs) if c["start_ms"] <= mid < c["end_ms"]), None)
            if k is not None:
                key = (cs[k]["id"], cs[k]["start_ms"], cs[k]["end_ms"])
                if key in tot:
                    tot[key]["lang"][g.get("lang") or "?"] += g["end_ms"] - g["start_ms"]
    out = []
    for key, t in tot.items():
        lang = max(t["lang"], key=t["lang"].get) if t["lang"] else None
        out.append({"key": key, "cls": t["cls"], "n": t["n"], "e": t["e"], "lang": lang})
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tune", nargs="+", type=Path, default=[])
    ap.add_argument("--heldout", nargs="+", type=Path, default=[])
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    manifest = {u.id: u for u in data.load_manifest("coshe-eval")}
    refs = {i: (u.text, u.duration_s) for i, u in manifest.items()}
    tune = json.loads((EVAL / "labels" / "coshe-tune-chunks.json").read_text(encoding="utf-8"))["chunks"]
    held = [{"id": c.id, "start_ms": c.start_ms, "end_ms": c.end_ms, "main": c.main, "kind": c.kind}
            for c in load_labels("coshe-eval", heldout=True)]
    L = ["WER per chunk class of finished decodes (Phase 5, item 4, step 3a); numbers only; written by eval/class_wer.py.",
         "Method: see the docstring of eval/class_wer.py. Figures compare runs on the same chunks; they are not a corpus WER.", ""]
    for title, runs, chunks in (("Tuning set (339 chunks; labels derived from the reference text, owner's ear check applied)", a.tune, tune),
                                ("Held-out set (10 chunks; hand labels)", a.heldout, held)):
        if not runs:
            continue
        L += [title, "run | " + " | ".join(f"{NAMES[k]}: chunks, words, WER %" for k in CLASSES) + " | all words, WER %"]
        scored = {r.name: score_run(r, chunks, refs) for r in runs}
        for name, rows in scored.items():
            cells = []
            for k in CLASSES:
                sel = [r for r in rows if r["cls"] == k]
                n, e = sum(r["n"] for r in sel), sum(r["e"] for r in sel)
                cells.append(f"{len(sel)}, {n}, {100 * e / n:.1f}" if n else f"{len(sel)}, 0, -")
            n, e = sum(r["n"] for r in rows), sum(r["e"] for r in rows)
            L.append(f"{name} | " + " | ".join(cells) + f" | {n}, {100 * e / n:.1f}")
        L.append("")
        # the same pure English chunks for every run: those that each pair run detected as English
        base = a.tune[0].name if runs is a.tune else a.heldout[0].name
        for name, rows in scored.items():
            if name == base:
                continue
            en_keys = {r["key"] for r in rows if r["cls"] == ("en", "pure") and r["lang"] == "en"}
            if len(en_keys) == 0 or "small" not in name:
                continue
            L.append(f"Pure English chunks that {name} detected as English ({len(en_keys)}): WER of each run on exactly those chunks")
            for n2, rows2 in scored.items():
                sel = [r for r in rows2 if r["key"] in en_keys]
                n, e = sum(r["n"] for r in sel), sum(r["e"] for r in sel)
                L.append(f"  {n2} | chunks {len(sel)} | words {n} | WER {100 * e / n:.1f}%" if n else f"  {n2} | none")
            L.append("")
    text = "\n".join(L).rstrip() + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
