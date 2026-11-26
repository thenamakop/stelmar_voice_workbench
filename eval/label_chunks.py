"""Chunk labels for the CoSHE-Eval tuning clips, derived from the reference text alone (eval/wbeval/chunklabeller.py).

    eval/.venv/Scripts/python -X utf8 -I eval/label_chunks.py validate [--out docs/evidence/phase-5/labeller-validation.txt]
    eval/.venv/Scripts/python -X utf8 -I eval/label_chunks.py build --run eval/results/<run> [--out eval/labels/coshe-tune-chunks.json]
    eval/.venv/Scripts/python -X utf8 -I eval/label_chunks.py sample [--n 10] [--seed 24112026] [--out docs/evidence/phase-5/ear-check.txt]

`validate` runs the labeller on the ten hand-labelled held-out chunks and counts agreement (a held-out run: it reads the
hand labels and the references of the three held-out clips, decodes nothing, tunes nothing). Pass: at least 9 of 10 on the
main language and 8 of 10 on pure against mixed.
`build` takes the chunk ranges from a finished decode of the tuning clips (the engine's segments: the ranges come from the
VAD and the chunker, which do not depend on the language rule) and labels each range from the reference words. The file
holds ids, time ranges, labels and shares: no text and no audio.
`sample` picks the random chunks of the owner's ear check and writes a plain list: clip id, start, end, proposed label.
`ear-check` reads the owner's marks (eval/labels/coshe-tune-ear-check.json, a person's data), counts agreement against the
same bar as `validate` (main 9 of 10, pure against mixed 8 of 10), applies the owner's corrections to the label file and
writes the result to docs/evidence/phase-5/ear-check.txt. `build` applies the same corrections, so a rebuilt label file
keeps them.
The written files are evidence: scripts write them, nobody edits them.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from wbeval import data  # noqa: E402
from wbeval.chunklabeller import (MIN_WORDS, PURE_BELOW, label_range, place_words, sample_for_ear_check,  # noqa: E402
                                  validate_against_hand_labels)
from wbeval.labels import load_labels  # noqa: E402

EVAL = Path(__file__).resolve().parent
TUNE_CHUNKS = EVAL / "labels" / "coshe-tune-chunks.json"
EAR_MARKS = EVAL / "labels" / "coshe-tune-ear-check.json"
MIN_CHUNK_S = 1.5


def apply_ear_check(chunks: list[dict]) -> int:
    """Sets a chunk's label to what the owner heard where the ear check disagreed with the derived label; returns how many."""
    changed = 0
    if not EAR_MARKS.exists():
        return changed
    for m in json.loads(EAR_MARKS.read_text(encoding="utf-8"))["marks"]:
        if m["mark"] != "disagree":
            continue
        hit = [c for c in chunks if (c["id"], c["start_ms"], c["end_ms"]) == (m["id"], m["start_ms"], m["end_ms"])]
        if len(hit) != 1:
            raise SystemExit(f"ear-check chunk {m['n']} ({m['id']} {m['start_ms']}-{m['end_ms']}) is not in the label file")
        c = hit[0]
        if (c["main"], c["kind"]) != (m["heard_main"], m["heard_kind"]):
            c["main"], c["kind"] = m["heard_main"], m["heard_kind"]
            changed += 1
        c["ear_check"] = "owner"
    return changed


def clock(ms: int) -> str:
    s = ms / 1000.0
    return f"{int(s // 60):02d}:{s % 60:05.2f}"


def write(path: Path | None, text: str) -> None:
    if path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))


def cmd_validate(args) -> int:
    hand = load_labels("coshe-eval", heldout=True)
    manifest = {u.id: u for u in data.load_manifest("coshe-eval")}
    refs = {lab.id: (manifest[lab.id].text, manifest[lab.id].duration_s) for lab in hand}
    r = validate_against_hand_labels(hand, refs)
    ok = r["main_agree"] >= 9 and r["kind_agree"] >= 8 and r["labelled"] == r["chunks"]
    L = ["Validation of the reference-only chunk labeller on the 10 held-out chunks (Phase 5, item 4; a held-out run: no decode).",
         "Method: eval/wbeval/chunklabeller.py; words spread over the utterance duration by characters, labelled by Latin against",
         f"Devanagari share, pure when the smaller share is under {PURE_BELOW:.0%}, at least {MIN_WORDS} words. No text in this file.", "",
         "n | clip | start-end ms | hand label (main, kind) | derived (main, kind) | latin share | main | kind"]
    for i, row in enumerate(r["rows"], 1):
        dm, dk = row["derived_main"] or "-", row["derived_kind"] or "-"
        L.append(f"{i} | {row['id']} | {row['start_ms']}-{row['end_ms']} | {row['hand_main']}, {row['hand_kind']} | {dm}, {dk} | "
                 f"{row['latin_share']} | {'agree' if dm == row['hand_main'] else 'DIFFER'} | "
                 f"{'agree' if dk == row['hand_kind'] else 'DIFFER'}")
    L += ["", f"labelled {r['labelled']} of {r['chunks']}; main language agrees {r['main_agree']} of {r['chunks']}; pure against mixed "
              f"agrees {r['kind_agree']} of {r['chunks']}",
          f"pass bar: 9 of 10 on main and 8 of 10 on pure against mixed: {'PASS' if ok else 'FAIL'}"]
    write(args.out, "\n".join(L) + "\n")
    return 0 if ok else 1


def cmd_build(args) -> int:
    run = Path(args.run)
    manifest = {u.id: u for u in data.load_manifest("coshe-eval")}
    tune_ids = {line.split("\t")[1].strip() for line in (EVAL / "clips" / "phase5-tune-coshe.txt").read_text(encoding="utf-8").splitlines()
                if line.strip() and not line.startswith("#")}
    chunks, skipped = [], {"too_short": 0, "too_few_words": 0}
    for line in (run / "utterances.jsonl").read_text(encoding="utf-8").splitlines():
        rec = json.loads(line)
        if rec["id"] not in tune_ids:
            raise SystemExit(f"{rec['id']} is not in the tuning set: the held-out clips never get tuning labels")
        words = place_words(manifest[rec["id"]].text, manifest[rec["id"]].duration_s * 1000.0)
        for seg in rec.get("segments", []):
            if (seg["end_ms"] - seg["start_ms"]) / 1000.0 < MIN_CHUNK_S:
                skipped["too_short"] += 1
                continue
            got = label_range(words, seg["start_ms"], seg["end_ms"])
            if got is None:
                skipped["too_few_words"] += 1
                continue
            chunks.append({"id": rec["id"], "start_ms": int(seg["start_ms"]), "end_ms": int(seg["end_ms"]), **got})
    corrected = apply_ear_check(chunks)
    out = {"dataset": "coshe-eval", "heldout": False,
           "about": "Chunk labels of the tuning clips, derived from the reference text alone by eval/label_chunks.py (spread over "
                    "the duration by characters, Latin against Devanagari share, pure under 15%). Ranges come from the engine's "
                    f"segments in {run.name}. Where the owner's ear check disagreed ({corrected} chunk(s)) the label is what the "
                    "owner heard (eval/labels/coshe-tune-ear-check.json). No text, no audio. Tuning only.",
           "from_run": run.name, "skipped": skipped, "chunks": chunks}
    path = args.out or TUNE_CHUNKS
    path.write_text(json.dumps(out, indent=1) + "\n", encoding="utf-8")
    by = {}
    for c in chunks:
        by[(c["main"], c["kind"])] = by.get((c["main"], c["kind"]), 0) + 1
    print(f"{len(chunks)} chunks labelled, skipped {skipped}; by (main, kind): {dict(sorted(by.items()))}; written {path}")
    return 0


def cmd_sample(args) -> int:
    chunks = json.loads(TUNE_CHUNKS.read_text(encoding="utf-8"))["chunks"]
    pick = sample_for_ear_check(chunks, args.n, args.seed)
    L = [f"Ear check of the chunk labels ({len(pick)} random chunks of the tuning clips; seed {args.seed}).",
         "Listen to each chunk (the clip's audio between start and end) and mark agree or disagree with the proposed label.",
         "main = the language that leads the chunk; pure = one language only; mixed = both languages.", "",
         "n | clip | start | end | proposed main | proposed kind | agree / disagree"]
    for i, c in enumerate(pick, 1):
        L.append(f"{i} | {c['id']} | {clock(c['start_ms'])} | {clock(c['end_ms'])} | {c['main']} | {c['kind']} |")
    write(args.out, "\n".join(L) + "\n")
    return 0


def cmd_ear_check(args) -> int:
    marks = json.loads(EAR_MARKS.read_text(encoding="utf-8"))
    raw = json.loads(TUNE_CHUNKS.read_text(encoding="utf-8"))
    rows = marks["marks"]
    main_ok = sum(1 for m in rows if m["heard_main"] == m["proposed_main"])
    kind_ok = sum(1 for m in rows if m["heard_kind"] == m["proposed_kind"])
    ok = main_ok >= 9 and kind_ok >= 8 and len(rows) == 10
    changed = apply_ear_check(raw["chunks"])
    if changed:
        TUNE_CHUNKS.write_text(json.dumps(raw, indent=1) + "\n", encoding="utf-8")
    L = [f"Ear check of the chunk labels ({len(rows)} random chunks of the tuning clips; seed {marks['seed']}); marks by {marks['by']}, "
         f"{marks['date']}; written by eval/label_chunks.py ear-check from {EAR_MARKS.name}.",
         "main = the language that leads the chunk; pure = one language only; mixed = both languages.",
         "A mark of disagree sets the chunk's label to what the owner heard; where the owner did not change the main language, "
         "the main language stays.", "",
         "n | clip | start | end | proposed (main, kind) | latin share | mark | owner's label (main, kind)"]
    for m in rows:
        L.append(f"{m['n']} | {m['id']} | {clock(m['start_ms'])} | {clock(m['end_ms'])} | {m['proposed_main']}, {m['proposed_kind']} | "
                 f"{m['latin_share']} | {m['mark']} | {m['heard_main']}, {m['heard_kind']}")
    miss = [m for m in rows if m["heard_kind"] != m["proposed_kind"]]
    L += ["", f"main language agrees {main_ok} of {len(rows)}; pure against mixed agrees {kind_ok} of {len(rows)}",
          f"pass bar: 9 of 10 on main and 8 of 10 on pure against mixed: {'PASS' if ok else 'FAIL'}",
          f"label file {TUNE_CHUNKS.name}: {sum(1 for m in rows if m['mark'] == 'disagree')} chunk(s) carry the owner's label "
          "(`build` keeps the corrections)"]
    for m in miss:
        L.append(f"disagreement {m['n']}: the labeller said {m['proposed_kind']} ({m['latin_share']} Latin share, so the smaller "
                 f"share was {min(m['latin_share'], 1 - m['latin_share']):.3f}, under the {PURE_BELOW:.0%} limit) and the owner hears "
                 f"{m['heard_kind']}")
    write(args.out, "\n".join(L) + "\n")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    v = sub.add_parser("validate")
    v.add_argument("--out", type=Path)
    v.set_defaults(fn=cmd_validate)
    b = sub.add_parser("build")
    b.add_argument("--run", required=True)
    b.add_argument("--out", type=Path)
    b.set_defaults(fn=cmd_build)
    s = sub.add_parser("sample")
    s.add_argument("--n", type=int, default=10)
    s.add_argument("--seed", type=int, default=24112026)
    s.add_argument("--out", type=Path)
    s.set_defaults(fn=cmd_sample)
    e = sub.add_parser("ear-check")
    e.add_argument("--out", type=Path)
    e.set_defaults(fn=cmd_ear_check)
    args = ap.parse_args()
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
