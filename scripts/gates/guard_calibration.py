"""Calibrates the loop threshold of the engine's compression guard (Phase 2, PLAN section 6, 10 Nov 2026).

    eval/.venv/Scripts/python scripts/gates/guard_calibration.py \
        --attempts D:/STELMAR-Gate/guard-calibration/attempts.jsonl --out docs/evidence/phase-2/guard-calibration.txt

Measures, with the engine's own code (crates/wb-engine-core/examples/guard_calibrate.rs, so Python cannot disagree
with Rust about zlib):
  ratio       characters per compressed byte, each distinct character first mapped to one byte (script-independent)
  ratio_utf8  whisper's measure, UTF-8 bytes per compressed byte (the one the guard used until 10 Nov 2026)
on every reference transcript available (CoSHE-Eval, FLEURS en, hi, ur), both whole and cut into windows of
25 and 50 words (a chunk is at most 15 s of speech, about 20 to 60 words), and on the known loop outputs.
Known loops are the decode attempts of the engine whose text looped, taken from the development attempt log
(WB_ATTEMPT_LOG) of the three Hinglish clips that lost speech; they are chosen by a rule that does not look at
the new measure: the attempt reached the token limit, or the old byte measure was over 2.4 AND the text shows
a repeated 1 to 6 word phrase of at least 4 repeats (so a clip of real speech that merely compresses well is
not a loop). Synthetic loops (a reference phrase repeated 6 to 20 times in each script) are reported separately.

The threshold is the geometric middle between the highest reference value and the lowest known-loop value;
the run FAILS when they overlap. The evidence holds counts, percentiles and numbers only, no transcript text.
"""

from __future__ import annotations

import argparse
import json
import math
import random
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

from wbeval.data import load_manifest  # noqa: E402

SETS = ("coshe-eval", "fleurs-en", "fleurs-hi", "fleurs-ur")


def run_measure(rows: list[tuple[str, str, str]]) -> list[dict]:
    """rows: (id, set, text). Returns measures per row, in order."""
    exe = subprocess.run(["cargo", "build", "--release", "-p", "wb-engine-core", "--example", "guard_calibrate"],
                         cwd=REPO, capture_output=True, text=True)
    if exe.returncode != 0:
        raise SystemExit(exe.stderr[-800:])
    binary = REPO / "target" / "release" / "examples" / "guard_calibrate.exe"
    payload = "".join(f"{i}\t{s}\t{' '.join(t.split())}\n" for i, s, t in rows)
    r = subprocess.run([str(binary)], input=payload, capture_output=True, text=True, encoding="utf-8")
    out = []
    for line in r.stdout.splitlines():
        i, s, chars, words, ratio, ratio_utf8, rep, *rest = line.split("\t")
        out.append({"id": i, "set": s, "chars": int(chars), "words": int(words), "ratio": float(ratio),
                    "ratio_utf8": float(ratio_utf8), "rep": int(rep), "letters": int(rest[0]) if rest else 0})
    assert len(out) == len(rows), (len(out), len(rows))
    return out


def windows(text: str, n: int) -> list[str]:
    w = text.split()
    return [" ".join(w[i:i + n]) for i in range(0, len(w) - n + 1, n)]


def pct(xs: list[float], p: float) -> float:
    xs = sorted(xs)
    if not xs:
        return float("nan")
    k = (len(xs) - 1) * p / 100
    lo, hi = math.floor(k), math.ceil(k)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def repeated_phrase(text: str) -> bool:
    """A 1 to 6 word phrase repeated back to back at least 4 times."""
    w = text.split()
    for n in range(1, 7):
        run, i = 1, 0
        while i + 2 * n <= len(w):
            if w[i:i + n] == w[i + n:i + 2 * n]:
                run += 1
                if run >= 4:
                    return True
            else:
                run = 1
            i += n
    return False


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--attempts", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--chosen", type=float, help="the threshold the engine uses; reported against the data")
    args = ap.parse_args()

    refs: list[tuple[str, str, str]] = []
    for ds in SETS:
        for u in load_manifest(ds):
            refs.append((u.id, ds, u.text))
    rows = [(i, f"{s}|whole", t) for i, s, t in refs]
    for i, s, t in refs:
        for n in (25, 50):
            for k, w in enumerate(windows(t, n)):
                rows.append((f"{i}#{n}.{k}", f"{s}|w{n}", w))
    ref_meas = run_measure(rows)

    attempts = [json.loads(x) for x in Path(args.attempts).read_text(encoding="utf-8").splitlines() if x.strip()]
    loops, legit = [], []
    for a in attempts:
        if a["step"] != "first" and not a["step"].startswith(("a_", "b_")):
            continue
        is_loop = a["truncated"] or (a["ratio_utf8"] > 2.4 and repeated_phrase(a["text"]))
        (loops if is_loop else legit).append(a)
    loop_meas = run_measure([(f"{a['source']}:{a['chunk']}:{a['step']}", "loop", a["text"]) for a in loops])
    legit_meas = run_measure([(f"{a['source']}:{a['chunk']}:{a['step']}", "decoded", a["text"]) for a in legit])

    rng = random.Random(20261110)
    syn_rows = []
    for ds in SETS:
        pool = [t for _, s, t in refs if s == ds and len(t.split()) >= 12]
        for k in range(60):
            t = rng.choice(pool).split()
            n = rng.choice((1, 2, 3, 4, 6))
            phrase = " ".join(t[:n])
            syn_rows.append((f"syn{k}", f"synthetic-{ds}", " ".join([phrase] * rng.randint(6, 20))))
    syn_meas = run_measure(syn_rows)

    lines = [
        "Calibration of the loop threshold of the compression guard (Phase 2, 10 Nov 2026)",
        "measure `ratio`: characters per compressed byte, each distinct character first mapped to one byte (script-independent);",
        "measure `utf8`: whisper's UTF-8 byte measure, the one the guard used before, shown for comparison. Computed by the engine's own code.",
        "no transcript text below: counts, percentiles and numbers only",
        "",
        "References (all of them, none cut off): utterances; windows of 25 and 50 words stand in for chunks of at most 15 s",
        "set | cut | n | ratio p50 | p90 | p99 | p99.9 | max | utf8 p50 | p99 | max | utf8 over 2.4 (count, share)",
    ]
    ref_max = 0.0
    for ds in SETS:
        for cut in ("whole", "w25", "w50"):
            m = [x for x in ref_meas if x["set"] == f"{ds}|{cut}" and x["chars"] >= 20]
            if not m:
                continue
            r = [x["ratio"] for x in m]
            u = [x["ratio_utf8"] for x in m]
            ref_max = max(ref_max, max(r))
            over = sum(1 for v in u if v > 2.4)
            lines.append(f"{ds} | {cut} | {len(m)} | {pct(r, 50):.2f} | {pct(r, 90):.2f} | {pct(r, 99):.2f} | {pct(r, 99.9):.2f} | {max(r):.2f} | "
                         f"{pct(u, 50):.2f} | {pct(u, 99):.2f} | {max(u):.2f} | {over} ({100 * over / len(m):.1f}%)")
    lines += ["", f"highest `ratio` of any reference (whole or window, 20 characters or more): {ref_max:.3f}"]

    loop_r = [x["ratio"] for x in loop_meas]
    lines += ["", f"Known loops from the decode attempts of the three Hinglish clips (rule above): {len(loop_meas)} attempts"]
    for x, a in zip(loop_meas, loops):
        lines.append(f"  {x['id']} | {x['chars']} characters | ratio {x['ratio']:.2f} | utf8 {x['ratio_utf8']:.2f} | repetition guard {x['rep']}")
    lines += ["", f"Other decode attempts (not loops by the rule): {len(legit_meas)}; ratio max {max([x['ratio'] for x in legit_meas] or [0]):.2f}, "
              f"utf8 max {max([x['ratio_utf8'] for x in legit_meas] or [0]):.2f}; those over utf8 2.4:"]
    for x in legit_meas:
        if x["ratio_utf8"] > 2.4:
            lines.append(f"  {x['id']} | {x['chars']} characters | ratio {x['ratio']:.2f} | utf8 {x['ratio_utf8']:.2f}")

    syn_by = {}
    for x in syn_meas:
        syn_by.setdefault(x["set"], []).append(x["ratio"])
    lines += ["", "Synthetic loops (a reference phrase of 1 to 6 words repeated 6 to 20 times), 60 per set: lowest ratio"]
    for k, v in syn_by.items():
        lines.append(f"  {k}: lowest {min(v):.2f}, median {pct(v, 50):.2f}")
    syn_low = min(min(v) for v in syn_by.values())

    ok = bool(loop_r) and max(ref_max, 0) < min(loop_r)
    lines += [""]
    if not ok:
        lines.append(f"RESULT: FAIL: the references reach {ref_max:.3f} and the lowest known loop is {min(loop_r) if loop_r else float('nan'):.3f}: no threshold separates them with this measure")
    else:
        t = math.sqrt(ref_max * min(loop_r))
        lines.append(f"threshold: geometric middle of the highest reference ({ref_max:.3f}) and the lowest known loop ({min(loop_r):.3f}) = {t:.2f}")
        lines.append(f"at {t:.2f}: references flagged 0; known loops flagged {sum(1 for v in loop_r if v > t)} of {len(loop_r)}; synthetic loops flagged "
                     f"{sum(1 for v in sum(syn_by.values(), []) if v > t)} of {sum(len(v) for v in syn_by.values())} (lowest {syn_low:.2f})")
        if args.chosen is not None:
            lines.append(f"threshold used by the engine: {args.chosen:.2f}: references flagged {sum(1 for x in ref_meas if x['chars'] >= 20 and x['ratio'] > args.chosen)}; "
                         f"known loops flagged {sum(1 for v in loop_r if v > args.chosen)} of {len(loop_r)}")
        lines.append("RESULT: separable")
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
