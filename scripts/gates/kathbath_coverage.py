"""Coverage check of the Kathbath references (Phase 3, 13 Nov 2026).

    eval/.venv/Scripts/python scripts/gates/kathbath_coverage.py --out docs/evidence/phase-3/kathbath-coverage.txt

Computes the engine's coverage rate (letters and digits per second of VAD speech, as
scripts/gates/coverage_calibration.py does) on the reference transcripts of the 100 Kathbath hi and 100
Kathbath ur utterances of eval/clips/phase3-kathbath-*.txt, and counts how many fall under the current floors
(guards::COVERAGE_FLOOR). Utterances with under 3 s of speech are not judged by the engine's check and are
counted apart. The evidence holds numbers and ids only, no transcript text. Speech seconds are cached with the
Phase 2 calibration cache in D:/STELMAR-Gate/coverage/speech-seconds.json.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from coverage_calibration import MIN_SPEECH_S, speech_seconds  # noqa: E402
from guard_calibration import pct, run_measure  # noqa: E402
from wbeval.data import load_manifest  # noqa: E402

FLOORS = {"en": 5.79, "hi": 5.03, "ur": 7.70}
SETS = {"kathbath-hi": "hi", "kathbath-ur": "ur"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--scratch", default="D:/STELMAR-Gate/coverage")
    args = ap.parse_args()
    scratch = Path(args.scratch)
    scratch.mkdir(parents=True, exist_ok=True)
    cache_path = scratch / "speech-seconds.json"
    cache = json.loads(cache_path.read_text(encoding="utf-8")) if cache_path.exists() else {}
    lines = [
        "Coverage check of the Kathbath references (Phase 3, 13 Nov 2026)",
        "rate = letters and digits per second of VAD speech (engine code: guards::letter_count; speech seconds from `wb-engine chunks --greedy`)",
        "sets: the 100 Kathbath test-unknown utterances per language of eval/clips/phase3-kathbath-hi.txt and -ur.txt",
        "no transcript text below: ids, counts and numbers only",
        "",
        f"current floors: {FLOORS}",
        "set | language | utterances | judged (3 s of speech or more) | min | p1 | p5 | p50 | p95 | below the floor | share",
    ]
    total_below = total_judged = 0
    detail = []
    for ds, lang in SETS.items():
        ids = [x.strip().split("	")[-1] for x in (REPO / "eval" / "clips" / f"phase3-{ds}.txt").read_text(encoding="utf-8").splitlines() if x.strip() and not x.startswith("#")]
        d = REPO / "eval" / "data" / ds
        by_id = {u.id: u for u in load_manifest(ds)}
        rows, keys = [], []
        for i in ids:
            u = by_id[i]
            key = f"{ds}/{i}"
            if key not in cache:
                cache[key] = speech_seconds(d / u.audio)
            rows.append((key, ds, u.text))
            keys.append(key)
        cache_path.write_text(json.dumps(cache), encoding="utf-8")
        meas = run_measure(rows)
        rates, below, short = [], [], 0
        for key, m in zip(keys, meas):
            s = cache[key]
            if s < MIN_SPEECH_S:
                short += 1
                continue
            r = m["letters"] / s
            rates.append(r)
            if r < FLOORS[lang]:
                below.append((key.split("/", 1)[1], r))
        total_below += len(below)
        total_judged += len(rates)
        lines.append(
            f"{ds} | {lang} | {len(ids)} | {len(rates)} | {min(rates):.2f} | {pct(rates, 1):.2f} | {pct(rates, 5):.2f} | "
            f"{pct(rates, 50):.2f} | {pct(rates, 95):.2f} | {len(below)} | {100 * len(below) / len(rates):.1f}%"
        )
        detail.append(f"{ds}: {short} utterance(s) under 3 s of speech (not judged); lowest reference rate {min(rates):.2f}; "
                      f"below the floor {FLOORS[lang]:.2f}: " + (", ".join(f"{i} at {r:.2f}" for i, r in below) or "none"))
    share = 100 * total_below / max(1, total_judged)
    lines += ["", *detail, "", f"references below the floors: {total_below} of {total_judged} judged = {share:.1f}% (limit 1%)",
              "RESULT: " + ("MORE than 1% below the floors: recalibrate the floors with Kathbath included" if share > 1.0
                            else "1% or fewer below the floors: the floors stand; flagged decodes are real low coverage")]
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
