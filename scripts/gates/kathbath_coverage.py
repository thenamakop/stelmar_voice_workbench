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

FLOORS = {"en": 5.79, "hi": 5.03, "ur": 7.70}  # Phase 2 (11 Nov 2026), the floors this check was written against
CURRENT = {"en": 5.79, "hi": 4.28, "ur": 4.43}  # guards::COVERAGE_FLOOR now (hi and ur changed on 13 Nov 2026; ur was 5.11 for a few hours)
SETS = {"kathbath-hi": "hi", "kathbath-ur": "ur"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--scratch", default="D:/STELMAR-Gate/coverage")
    ap.add_argument("--full", action="store_true",
                    help="every utterance of the Kathbath test-unknown sets instead of the 100 per language; also reports the recalibrated floors")
    ap.add_argument("--workers", type=int, default=8)
    args = ap.parse_args()
    scratch = Path(args.scratch)
    scratch.mkdir(parents=True, exist_ok=True)
    cache_path = scratch / "speech-seconds.json"
    cache = json.loads(cache_path.read_text(encoding="utf-8")) if cache_path.exists() else {}
    title = ("Coverage check of the FULL Kathbath test-unknown sets (Phase 3, 13 Nov 2026)" if args.full
             else "Coverage check of the Kathbath references (Phase 3, 13 Nov 2026)")
    lines = [
        title,
        "rate = letters and digits per second of VAD speech (engine code: guards::letter_count; speech seconds from `wb-engine chunks --greedy`)",
        ("sets: every utterance of eval/data/kathbath-hi and kathbath-ur (the official test-unknown files), reference text and VAD speech time only, no decoding"
         if args.full else
         "sets: the 100 Kathbath test-unknown utterances per language of eval/clips/phase3-kathbath-hi.txt and -ur.txt"),
        "no transcript text below: ids, counts and numbers only",
        "",
        f"current floors: {FLOORS}",
        "set | language | utterances | judged (3 s of speech or more) | min | p1 | p5 | p50 | p95 | below the floor | share",
    ]
    total_below = total_judged = 0
    all_rates: dict[str, list[float]] = {}
    detail = []
    for ds, lang in SETS.items():
        d = REPO / "eval" / "data" / ds
        by_id = {u.id: u for u in load_manifest(ds)}
        if args.full:
            ids = list(by_id)
        else:
            ids = [x.strip().split("	")[-1] for x in (REPO / "eval" / "clips" / f"phase3-{ds}.txt").read_text(encoding="utf-8").splitlines() if x.strip() and not x.startswith("#")]
        rows, keys = [], []
        todo = [i for i in ids if f"{ds}/{i}" not in cache]
        if todo:
            from concurrent.futures import ThreadPoolExecutor
            with ThreadPoolExecutor(max_workers=args.workers) as pool:
                for n, (i, sec) in enumerate(zip(todo, pool.map(lambda i: speech_seconds(d / by_id[i].audio), todo)), 1):
                    cache[f"{ds}/{i}"] = sec
                    if n % 500 == 0:
                        cache_path.write_text(json.dumps(cache), encoding="utf-8")
                        print(f"{ds}: {n} of {len(todo)} speech times", file=sys.stderr, flush=True)
        for i in ids:
            u = by_id[i]
            key = f"{ds}/{i}"
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
        all_rates[lang] = rates
        total_below += len(below)
        total_judged += len(rates)
        lines.append(
            f"{ds} | {lang} | {len(ids)} | {len(rates)} | {min(rates):.2f} | {pct(rates, 1):.2f} | {pct(rates, 5):.2f} | "
            f"{pct(rates, 50):.2f} | {pct(rates, 95):.2f} | {len(below)} | {100 * len(below) / len(rates):.1f}%"
        )
        detail.append(f"{ds}: {short} utterance(s) under 3 s of speech (not judged); lowest reference rate {min(rates):.2f}; "
                      f"below the floor {FLOORS[lang]:.2f}: " + (", ".join(f"{i} at {r:.2f}" for i, r in (sorted(below, key=lambda x: x[1])[:10] if args.full else below)) or "none")
                      + (f" (the {len(below)} counted; the 10 lowest listed)" if args.full and len(below) > 10 else ""))
    share = 100 * total_below / max(1, total_judged)
    lines += ["", *detail, "", f"references below the floors: {total_below} of {total_judged} judged = {share:.1f}% (limit 1%)",
              "RESULT: " + ("MORE than 1% below the floors: recalibrate the floors with Kathbath included" if share > 1.0
                            else "1% or fewer below the floors: the floors stand; flagged decodes are real low coverage")]
    if args.full:
        # The Phase 2 floors are 0.9 x the lowest reference rate of the language over the sets it used (FLEURS); with Kathbath
        # the lowest rate is the smaller of the two, so the floor is 0.9 x min(Phase 2 lowest, Kathbath lowest).
        lines += ["", "recalibration (same method: 0.9 x the lowest reference rate, so no reference is under the floor)",
                  "language | Phase 2 floor | engine floor now | Phase 2 lowest rate (floor / 0.9) | Kathbath lowest | new floor | under the Phase 2 floor | under the engine floor now | under the new floor | p0.1 | p1"]
        for lang, rates in all_rates.items():
            old = FLOORS[lang]
            lowest = min(old / 0.9, min(rates))
            new = round(0.9 * lowest, 2)
            under = sum(1 for r in rates if r < new)
            lines.append(f"{lang} | {old:.2f} | {CURRENT[lang]:.2f} | {old / 0.9:.2f} | {min(rates):.2f} | {new:.2f} | "
                         f"{sum(1 for r in rates if r < old)} | {sum(1 for r in rates if r < CURRENT[lang])} | {under} | {pct(rates, 0.1):.2f} | {pct(rates, 1):.2f}")
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
