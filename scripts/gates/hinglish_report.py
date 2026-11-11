"""Report of the Hinglish engine gate (restated 11 Nov 2026, docs/GATES.md row 5a to 2).

    eval/.venv/Scripts/python scripts/gates/hinglish_report.py \
        --run eval/results/phase2-wb-hinglish-q5_0-coshe \
        --attempts D:/STELMAR-Gate/guard-calibration/gate-attempts2.jsonl \
        --out docs/evidence/phase-2/hinglish-report.txt

Engine part (gated): 0 broken utterances and 0 silent loss. Silent loss is a chunk of speech that left
neither text nor a visible flag: no segment overlaps it and it was not dropped as non-speech (an event) or
empty (an event). Model part (reported, carried to Phase 5): chunks that ended `unclear` or `low_coverage`,
the harness's truncated-utterance count, and the ladder step that rescued each chunk.

Inputs: the run's utterances.jsonl (segments carry their flags once the harness keeps them) and the
development attempt log written with WB_ATTEMPT_LOG during the same run (it has transcript text and stays
in scratch; this report prints none of it).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

import metrics  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--run", required=True)
    ap.add_argument("--attempts", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    run = Path(args.run)
    summary = json.loads((run / "summary.json").read_text(encoding="utf-8"))
    utts = {}
    for line in (run / "utterances.jsonl").read_text(encoding="utf-8").splitlines():
        if line.strip():
            u = json.loads(line)
            utts[u["id"]] = u
    by_chunk: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for line in Path(args.attempts).read_text(encoding="utf-8").splitlines():
        if line.strip():
            a = json.loads(line)
            by_chunk[(a["source"], a["chunk"])].append(a)

    broken = [i for i, u in utts.items() if "\ufffd" in u["hyp"] or "??" in u["hyp"]]
    truncated = []
    for i, u in utts.items():
        s = metrics.prepare(i, u["ref"], u["hyp"], u["lang"], u["mixed"])
        if metrics.truncated(s):
            truncated.append(i)

    n_chunks = len(by_chunk)
    good_first = skipped = empty = silent = 0
    rescued: list[tuple[str, int, int, int, str]] = []
    unclear: list[tuple[str, int, int, int]] = []
    low: list[tuple[str, int, int, int]] = []
    silent_list: list[tuple[str, int, int, int]] = []
    flag_counts: dict[str, int] = defaultdict(int)
    for u in utts.values():
        for s in u["segments"]:
            for f in s.get("flags", []):
                flag_counts[f] += 1
    for (src, chunk), att in sorted(by_chunk.items()):
        first = next(a for a in att if a["step"] == "first")
        span = (first["start_ms"], first["end_ms"])
        verdict = first["verdict"]
        segs = [s for s in utts[src]["segments"] if min(s["end_ms"], span[1]) - max(s["start_ms"], span[0]) > 0] if src in utts else []
        if verdict == "Good":
            good_first += 1
            if first["chars"] == 0:
                empty += 1  # an empty decode of speech: a `chunk_empty` warning event
            continue
        if verdict.startswith("Skip"):
            skipped += 1  # a `segment_dropped` warning event
            continue
        # Fail(...): the ladder ran.
        a_good = [a for a in att if a["step"].startswith("a_t") and a["verdict"] == "Good"]
        b_good = [a for a in att if a["step"].startswith("b_") and a["verdict"] == "Good"]
        flags = {f for s in segs for f in s.get("flags", [])}
        if a_good:
            rescued.append((src, chunk, *span, f"step a at temperature {a_good[0]['step'][3:]}"))
        elif b_good:
            rescued.append((src, chunk, *span, "step b (cut at the deepest pause)"))
        if "unclear" in flags:
            unclear.append((src, chunk, *span))
        if "low_coverage" in flags:
            low.append((src, chunk, *span))
        if not segs:
            silent += 1
            silent_list.append((src, chunk, *span))

    lines = [
        "Hinglish engine gate report (Phase 2, 11 Nov 2026)",
        f"run: {run.name}; WER {100 * summary['wer']:.1f}, CER {100 * summary['cer']:.1f}; RTF {summary['rtf']:.2f} (information only: the attempt log is on and failed chunks are decoded again)",
        "no transcript text below: counts, clip names and times only",
        "",
        "ENGINE PART (gated): 0 broken utterances and 0 silent loss",
        f"- broken utterances (U+FFFD or '??'): {len(broken)}" + ("" if not broken else " " + ", ".join(broken)),
        f"- chunks: {n_chunks}; decoded cleanly at the first attempt: {good_first} (of which empty, with a chunk_empty event: {empty}); dropped as non-speech with a segment_dropped event: {skipped}; sent through the retry ladder: {n_chunks - good_first - skipped}",
        f"- silent loss (a speech chunk with no segment and no event): {silent}" + ("" if not silent_list else "; " + "; ".join(f"{s}:{c} {a}-{b} ms" for s, c, a, b in silent_list)),
        f"- ENGINE PART: {'PASS' if not broken and not silent else 'FAIL'}",
        "",
        "MODEL PART (reported; carried in the Phase 5 row)",
        f"- chunks that ended unclear: {len(unclear)}" + ("" if not unclear else "; " + "; ".join(f"{s}:{c} {a}-{b} ms" for s, c, a, b in unclear)),
        f"- chunks that ended low_coverage (text kept and flagged): {len(low)}" + ("" if not low else "; " + "; ".join(f"{s}:{c} {a}-{b} ms" for s, c, a, b in low)),
        f"- truncated utterances by the harness's rule (under 0.7 hypothesis words per reference word, or 20 or more reference words deleted in a row): {len(truncated)}" + ("" if not truncated else " " + ", ".join(truncated)),
        f"- segment flags in the output: {dict(sorted(flag_counts.items()))}",
        "",
        "Ladder: chunks rescued and by which step",
    ]
    lines += [f"  {s}:{c} {a}-{b} ms: {how}" for s, c, a, b, how in rescued] or ["  none"]
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0 if not broken and not silent else 1


if __name__ == "__main__":
    sys.exit(main())
