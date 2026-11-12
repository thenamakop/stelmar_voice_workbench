"""Calibrates the coverage floor of the engine (PLAN section 6, coverage check, 11 Nov 2026).

    eval/.venv/Scripts/python scripts/gates/coverage_calibration.py \
        --attempts D:/STELMAR-Gate/guard-calibration/gate-attempts.jsonl --out docs/evidence/phase-2/coverage-calibration.txt

The rate is letters and digits per second of VAD speech (`guards::letter_count`, counted by the engine's own
code through crates/wb-engine-core/examples/guard_calibrate.rs; speech seconds from `wb-engine chunks
--greedy`, the same VAD and chunker as `batch`). It is computed on every reference transcript (CoSHE-Eval as
Hindi, FLEURS en, hi and ur) over the whole utterance. The floor of a language is 0.9 times the lowest
reference rate of that language, so it lies below the whole reference distribution. The chunk-level check
uses the engine's own accepted decodes (the attempt log of the Hinglish run): how many normal chunks of 3 s
of speech or more fall below the floor, and whether the known short chunk (audio_1444 chunk 1) does.

Speech seconds are cached in SCRATCH (D:/STELMAR-Gate/coverage/speech-seconds.json) so a stopped run resumes.
The evidence holds counts, percentiles and numbers only, no transcript text.
Heavy (about 3,300 VAD runs): run it when no timing-sensitive run is in progress.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from guard_calibration import pct, run_measure  # noqa: E402
from wbeval.data import load_manifest  # noqa: E402

ENGINE = Path(os.environ.get("WB_ENGINE", REPO / "src-tauri" / "binaries" / "wb-engine-avx2-x86_64-pc-windows-msvc.exe"))
SETS = {"coshe-eval": "hi", "fleurs-en": "en", "fleurs-hi": "hi", "fleurs-ur": "ur"}
FACTOR = 0.9
MIN_SPEECH_S = 3.0  # guards::COVERAGE_MIN_SPEECH_S


def speech_seconds(wav: Path) -> float:
    with tempfile.TemporaryDirectory() as t:
        out = Path(t) / "c.json"
        r = subprocess.run([str(ENGINE), "chunks", "--file", str(wav), "--greedy", "--out", str(out)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        if r.returncode != 0:
            raise SystemExit(f"wb-engine chunks failed on {wav.name}: {r.stderr[-300:]}")
        doc = json.loads(out.read_text(encoding="utf-8"))
    return sum((c["end_ms"] - c["start_ms"]) / 1000 * c["speech_ratio"] for c in doc["chunks"])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--attempts", required=True, help="attempt log of the Hinglish run (scratch; has text)")
    ap.add_argument("--out", required=True)
    ap.add_argument("--scratch", default="D:/STELMAR-Gate/coverage")
    ap.add_argument("--known", default="audio_1444:1", help="source:chunk of the case that must be caught")
    args = ap.parse_args()
    scratch = Path(args.scratch)
    scratch.mkdir(parents=True, exist_ok=True)
    cache_path = scratch / "speech-seconds.json"
    cache: dict[str, float] = json.loads(cache_path.read_text(encoding="utf-8")) if cache_path.exists() else {}

    rows, keys = [], []
    for ds in SETS:
        d = REPO / "eval" / "data" / ds
        for u in load_manifest(ds):
            key = f"{ds}/{u.id}"
            if key not in cache:
                cache[key] = speech_seconds(d / u.audio)
                if len(cache) % 100 == 0:
                    cache_path.write_text(json.dumps(cache), encoding="utf-8")
                    print(len(cache), "clips measured", flush=True)
            rows.append((key, ds, u.text))
            keys.append(key)
    cache_path.write_text(json.dumps(cache), encoding="utf-8")
    meas = run_measure(rows)
    rates: dict[str, list[float]] = {ds: [] for ds in SETS}
    for key, m in zip(keys, meas):
        s = cache[key]
        if s >= MIN_SPEECH_S:
            rates[m["set"]].append(m["letters"] / s)

    lines = [
        "Calibration of the coverage floor (Phase 2, 11 Nov 2026)",
        "rate = letters and digits per second of VAD speech (engine code: guards::letter_count; speech seconds from `wb-engine chunks --greedy`)",
        "no transcript text below: counts, percentiles and numbers only",
        "",
        "References, whole utterances with 3 s of speech or more",
        "set | language | n | min | p1 | p5 | p50 | p95",
    ]
    base: dict[str, float] = {}
    for ds, lang in SETS.items():
        r = rates[ds]
        lines.append(f"{ds} | {lang} | {len(r)} | {min(r):.2f} | {pct(r, 1):.2f} | {pct(r, 5):.2f} | {pct(r, 50):.2f} | {pct(r, 95):.2f}")
        base[lang] = min(base.get(lang, math.inf), min(r))
    floors = {lang: round(FACTOR * v, 2) for lang, v in base.items()}
    lines += ["", f"floor = {FACTOR} x the lowest reference rate of the language (below the whole reference distribution):"]
    for lang, f in sorted(floors.items()):
        lines.append(f"  {lang}: lowest reference {base[lang]:.2f}, floor {f:.2f}")
    lines.append("  Rust: pub const COVERAGE_FLOOR: &[(&str, f32)] = &[" + ", ".join(f'("{l}", {f:.2f})' for l, f in sorted(floors.items())) + "];")

    # Chunk level, from the engine's own accepted decodes.
    attempts = [json.loads(x) for x in Path(args.attempts).read_text(encoding="utf-8").splitlines() if x.strip()]
    firsts = [a for a in attempts if a["step"] == "first"]
    texts = [(f"{a['source']}:{a['chunk']}", "chunk", a["text"]) for a in firsts]
    cm = run_measure(texts)
    lines += ["", "Chunk level: the first decode of every chunk of the Hinglish run (language hi), chunks with 3 s of speech or more"]
    chunk_rates, known_rate, known_seen = [], None, False
    below = []
    for a, m in zip(firsts, cm):
        s = (a["end_ms"] - a["start_ms"]) / 1000 * a["speech_ratio"]
        if f"{a['source']}:{a['chunk']}" == args.known:
            known_seen = True
            known_rate = m["letters"] / s if s > 0 else 0.0
        if s < MIN_SPEECH_S or a["verdict"] != "Good":
            continue
        rate = m["letters"] / s
        chunk_rates.append(rate)
        if rate < floors["hi"]:
            below.append((a["source"], a["chunk"], rate))
    lines.append(f"  accepted chunks judged: {len(chunk_rates)}; rate min {min(chunk_rates):.2f}, p1 {pct(chunk_rates, 1):.2f}, p5 {pct(chunk_rates, 5):.2f}, p50 {pct(chunk_rates, 50):.2f}")
    lines.append(f"  accepted chunks below the hi floor ({floors['hi']:.2f}): {len(below)}" + ("" if not below else "; " + ", ".join(f"{s}:{c} at {r:.2f}" for s, c, r in below)))
    ok = known_seen and known_rate is not None and known_rate < floors["hi"]
    lines.append(f"  known case {args.known}: " + (f"rate {known_rate:.2f}" if known_seen else "not in the attempt log") + (f" -> {'caught' if ok else 'NOT caught'} by the floor {floors['hi']:.2f}"))
    lines += ["", "RESULT: " + ("floor catches the known case and flags no reference" if ok else "FAIL: the floor does not catch the known case; the check is not usable with this rule")]
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
