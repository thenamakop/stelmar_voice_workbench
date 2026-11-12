"""Language-detection confusion on pure Hindi and pure Urdu read speech (Phase 2, information only, 12 Nov 2026)

    eval/.venv/Scripts/python -I scripts/gates/detection_confusion.py --set kathbath-hi --langset en,hi,ur \
        --id-prefix tu_ --limit 100 --raw-out D:/STELMAR-Gate/dc/hi-enhiur.json
    (four cells: kathbath-hi and kathbath-ur, each under en,hi and en,hi,ur; they are independent and can run in parallel)
    eval/.venv/Scripts/python -I scripts/gates/detection_confusion.py --report D:/STELMAR-Gate/dc/*.json \
        --out docs/evidence/phase-2/detection-confusion-kathbath.txt

Runs `wb-engine detect` (the engine's own chunker and detector; no decoding) over the same seeded sample of
each set once per session language set (`en,hi` and `en,hi,ur`; detection is restricted to the session's
languages, as in a session) and reports, per spoken language and per language set, how many chunks were
detected as each language: hi->en, ur->en, hi->ur and ur->hi, each with the probability of that wrong
detection (p50, p90, max, count over 0.9). In an `en,hi` session Urdu is not an expected language, so an Urdu
chunk can only come out as `en` or `hi`; in an `en,hi,ur` session Hindi can come out as `ur` and the reverse,
and the transcript then switches script mid-session (Devanagari against right-to-left Nastaliq). Counts and
numbers only: no audio and no text in the evidence (Kathbath is evaluation-only data).

Not a gate. Heavy: run it when no timing-sensitive run is in progress.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from guard_calibration import pct  # noqa: E402
from wbeval.data import load_manifest  # noqa: E402

ENGINE = Path(os.environ.get("WB_ENGINE", REPO / "src-tauri" / "binaries" / "wb-engine-avx2-x86_64-pc-windows-msvc.exe"))


def detect(wav: Path, langs: str, models: str, threads: int) -> list[dict]:
    with tempfile.TemporaryDirectory() as t:
        out = Path(t) / "d.jsonl"
        r = subprocess.run(
            [str(ENGINE), "detect", "--file", str(wav), "--models", models, "--langs", langs, "--out", str(out),
             "--threads", str(threads), "--settings", json.dumps({"sentence_split": True})],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        if r.returncode != 0:
            raise SystemExit(f"wb-engine detect failed on {wav.name} ({r.returncode}): {r.stderr[-300:]}")
        return [json.loads(x) for x in out.read_text(encoding="utf-8").splitlines() if x.strip()]


def run_cell(args) -> dict:
    """One dataset under one session language set. Raw numbers only (no audio, no text)."""
    utts = [u for u in load_manifest(args.set) if u.id.startswith(args.id_prefix)]
    n_pool = len(utts)
    utts.sort(key=lambda u: u.id)
    random.Random(args.seed).shuffle(utts)
    utts = utts[: args.limit]
    langs = args.langset.split(",")
    cell = {"set": args.set, "langset": args.langset, "utterances": len(utts), "pool": n_pool, "id_prefix": args.id_prefix,
            "seed": args.seed, "engine": ENGINE.name, "models": args.models, "threads": args.threads,
            "chunks": {}, "to": {}, "wrong_probs": {}, "top_other": {}}
    for u in utts:
        label = u.lang
        for ch in detect(REPO / "eval" / "data" / args.set / u.audio, args.langset, args.models, args.threads):
            if ch["speech_s"] < args.min_speech_s:
                continue
            probs = {p["lang"]: p["prob"] for p in ch["probs"]}
            if not probs:
                continue
            best = max(probs, key=lambda k: probs[k])
            cell["chunks"][label] = cell["chunks"].get(label, 0) + 1
            cell["to"].setdefault(label, {})
            cell["to"][label][best] = cell["to"][label].get(best, 0) + 1
            top_any = (ch.get("top_any") or {}).get("lang")
            if top_any and top_any not in langs:
                cell["top_other"][label] = cell["top_other"].get(label, 0) + 1
            if best != label:
                cell["wrong_probs"].setdefault(label, {}).setdefault(best, []).append(probs[best])
    return cell


def report(cells: list[dict], out: Path) -> None:
    first = cells[0]
    samples = sorted({f"{c['set']}: {c['utterances']} utterances of {c['pool']} with id prefix '{c['id_prefix']}'" for c in cells})
    lines = [
        "Language-detection confusion on pure Hindi and pure Urdu read speech (Phase 2, information only, 12 Nov 2026)",
        f"engine: {first['engine']} detect; models {first['models']}; sentence_split on; chunks with at least 1 s of speech; threads {first['threads']}",
        f"sample (seed {first['seed']}, ids sorted then shuffled): " + "; ".join(samples) + "; no audio and no text below",
        "Detection is restricted to the session's languages. An Urdu chunk in an en,hi session can only come out as en or hi, because Urdu is not an expected language there.",
        "The probability columns are the probability of the WRONG language the chunk was assigned (p50, p90, max over those chunks; count above 0.9).",
        "",
    ]
    for langset in sorted({c["langset"] for c in cells}, key=lambda x: (len(x), x)):
        langs = langset.split(",")
        lines.append(f"== session languages {langset}")
        lines.append("spoken -> detected | chunks of the spoken language | chunks detected as that | share | wrong-detection probability p50 | p90 | max | over 0.9")
        for c in sorted((c for c in cells if c["langset"] == langset), key=lambda c: c["set"]):
            for label in sorted(c["chunks"]):
                n = c["chunks"][label]
                for tgt in langs:
                    k = c["to"].get(label, {}).get(tgt, 0)
                    w = c["wrong_probs"].get(label, {}).get(tgt, [])
                    tag = "(correct)" if tgt == label else "(flip)"
                    lines.append(
                        f"{label} -> {tgt} {tag} | {n} | {k} | {100 * k / n:.1f}% | "
                        + (f"{pct(w, 50):.2f} | {pct(w, 90):.2f} | {max(w):.2f} | {sum(v > 0.9 for v in w)}" if w else "n/a | n/a | n/a | 0")
                    )
                lines.append(f"   {label}: chunks whose most likely language among all of whisper's is not in {langset}: {c['top_other'].get(label, 0)} of {n}")
        lines.append("")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--set", help="one dataset under eval/data; the label is each utterance's manifest language")
    ap.add_argument("--langset", help="one session language set, e.g. en,hi or en,hi,ur")
    ap.add_argument("--raw-out", help="write this cell's raw numbers (JSON) here")
    ap.add_argument("--report", nargs="+", help="raw JSON files of the cells to merge into the evidence file (--out)")
    ap.add_argument("--id-prefix", default="", help="keep only utterance ids starting with this (tu_ = Kathbath test-unknown)")
    ap.add_argument("--limit", type=int, default=100, help="utterances per set")
    ap.add_argument("--seed", type=int, default=20261112)
    ap.add_argument("--models", default="en=medium")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--min-speech-s", type=float, default=1.0, help="ignore chunks with less speech than this")
    ap.add_argument("--out")
    args = ap.parse_args()
    if args.report:
        report([json.loads(Path(f).read_text(encoding="utf-8")) for f in args.report], Path(args.out))
        return 0
    cell = run_cell(args)
    Path(args.raw_out).write_text(json.dumps(cell, ensure_ascii=False), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
