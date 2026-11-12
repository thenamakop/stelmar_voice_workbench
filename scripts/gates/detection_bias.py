"""How often does the language detector prefer the wrong language? (Phase 2, information only, 11 Nov 2026)

    eval/.venv/Scripts/python scripts/gates/detection_bias.py --set fleurs-en --limit 100 --out docs/evidence/phase-2/detection-bias-fleurs-en.txt
    eval/.venv/Scripts/python scripts/gates/detection_bias.py --set coshe-eval --only-lang en --limit 100 ...
    eval/.venv/Scripts/python scripts/gates/detection_bias.py --set coshe-eval --only-lang hi --limit 100 ...

Runs `wb-engine detect` (the engine's own chunker and detector, restricted to the languages given, medium
by default; no decoding) over a seeded sample of an evaluation set and reports, per set and per label:
the number of chunks, the share detected as the other language, and the probabilities of those wrong
detections (percentiles and the count over 0.9). The label of a chunk is the language of its utterance
as the manifest gives it (`lang`); for CoSHE-Eval the `en` utterances are those whose reference is
entirely in Latin script (pure English by a speaker of Indian English), the `hi` ones are Hindi-matrix
code-switched. It also reports the share of chunks whose most likely language among ALL of whisper's is
neither of the two. Counts and numbers only: no audio and no text in the evidence (CoSHE-Eval and
Kathbath are evaluation-only data).

Information for the detection-rule comparison of Phase 5 (rules a to d, GATES.md row 2 to 5); not a gate.
Heavy: run it when no timing-sensitive run is in progress.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
import soundfile as sf

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from guard_calibration import pct  # noqa: E402
from wbeval.data import load_manifest  # noqa: E402

ENGINE = Path(os.environ.get("WB_ENGINE", REPO / "src-tauri" / "binaries" / "wb-engine-avx2-x86_64-pc-windows-msvc.exe"))


def level_dbfs(wav: Path) -> float:
    x, _ = sf.read(wav, dtype="float32", always_2d=True)
    r = float(np.sqrt(np.mean(np.square(x.mean(axis=1).astype(np.float64))))) if x.size else 0.0
    return 20 * math.log10(r) if r > 0 else -120.0


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


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--set", required=True, help="a dataset name under eval/data (fleurs-en, coshe-eval, kathbath-hi, ...)")
    ap.add_argument("--only-lang", help="keep only utterances whose manifest language is this")
    ap.add_argument("--limit", type=int, default=100)
    ap.add_argument("--seed", type=int, default=20261111)
    ap.add_argument("--langs", default="en,hi")
    ap.add_argument("--models", default="en=medium")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--min-speech-s", type=float, default=1.0, help="ignore chunks with less speech than this")
    ap.add_argument("--min-dbfs", type=float, help="skip utterances whose RMS level is below this (FLEURS English is very quiet: 451 of 647 clips are under -50 dBFS)")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    utts = [u for u in load_manifest(args.set) if not args.only_lang or u.lang == args.only_lang]
    n_manifest = len(utts)
    if args.min_dbfs is not None:
        utts = [u for u in utts if level_dbfs(REPO / "eval" / "data" / args.set / u.audio) >= args.min_dbfs]
    n_after_level = len(utts)
    utts.sort(key=lambda u: u.id)
    random.Random(args.seed).shuffle(utts)
    utts = utts[: args.limit]
    langs = args.langs.split(",")
    wrong: dict[str, list[float]] = {}
    total: dict[str, int] = {}
    wrong_n: dict[str, int] = {}
    other_top = 0
    n_all = 0
    margins: dict[str, list[float]] = {}
    for u in utts:
        label = u.lang
        for ch in detect(REPO / "eval" / "data" / args.set / u.audio, args.langs, args.models, args.threads):
            if ch["speech_s"] < args.min_speech_s:
                continue
            probs = {p["lang"]: p["prob"] for p in ch["probs"]}
            if not probs or label not in probs:
                continue
            n_all += 1
            total[label] = total.get(label, 0) + 1
            best = max(probs, key=lambda k: probs[k])
            top_any = (ch.get("top_any") or {}).get("lang")
            if top_any and top_any not in langs:
                other_top += 1
            margins.setdefault(label, []).append(probs[label] - max(v for k, v in probs.items() if k != label))
            if best != label:
                wrong_n[label] = wrong_n.get(label, 0) + 1
                wrong.setdefault(label, []).append(probs[best])

    lines = [
        f"Language-detection bias on {args.set}" + (f" ({args.only_lang} utterances)" if args.only_lang else "") + " (Phase 2, information only, 11 Nov 2026)",
        f"engine: {ENGINE.name} detect; models {args.models}; languages {args.langs}; sentence_split on; chunks with at least {args.min_speech_s:g} s of speech",
        f"sample: {len(utts)} utterances, seed {args.seed}, taken from the manifest sorted by id then shuffled" + (f" (level filter: RMS at least {args.min_dbfs:g} dBFS keeps {n_after_level} of {n_manifest})" if args.min_dbfs is not None else "") + "; no audio and no text below",
        "",
        "label | chunks | detected as the other language | share | wrong-detection probability p50 | p90 | max | over 0.9 | margin (label minus best other) p10 | p50 | p90",
    ]
    for label in sorted(total):
        w = wrong.get(label, [])
        m = margins[label]
        lines.append(
            f"{label} | {total[label]} | {wrong_n.get(label, 0)} | {100 * wrong_n.get(label, 0) / total[label]:.1f}% | "
            + (f"{pct(w, 50):.2f} | {pct(w, 90):.2f} | {max(w):.2f} | {sum(v > 0.9 for v in w)}" if w else "n/a | n/a | n/a | 0")
            + f" | {pct(m, 10):+.2f} | {pct(m, 50):+.2f} | {pct(m, 90):+.2f}"
        )
    lines += ["", f"chunks whose most likely language among all of whisper's is neither of {args.langs}: {other_top} of {n_all}"]
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
