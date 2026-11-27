"""Accuracy of the Hindi and Urdu live (near-live) models against the rolling settings of the same model on the same clips
(Phase 5, item 8; owner's night plan of 27 Nov 2026). Numbers only; no decoding.

    eval/.venv/Scripts/python -X utf8 -I eval/item8_report.py [--out docs/evidence/phase-5/item8-live-accuracy.txt]

Reads eval/results/item8-*/summary.json (runs `item8-<model>-<dataset>-<role>` of eval/runs.toml: `wb-engine` batch with the
role's settings, the language given). The near-live bar of PLAN section 6 is a WER no more than 1.5 times the rolling WER of the
same model on the same clips, and an RTF at or under 0.35 on the machine that runs it; the RTF here is the harness's (the whole
run, model load included, on the Legion's RTX 4070 with the CUDA engine), so it speaks for the Legion's GPU live gate only.
First-word latency and the share of segments revised after commit are not measurable yet: `wb-engine live` reports speech
boundaries only (it decodes from Phase 7), which the report states.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

EVAL = Path(__file__).resolve().parent
BAR_RATIO = 1.5
BAR_RTF = 0.35


def load() -> dict[tuple[str, str, str], dict]:
    out = {}
    for d in sorted((EVAL / "results").glob("item8-*")):
        f = d / "summary.json"
        if not f.exists():
            continue
        s = json.loads(f.read_text(encoding="utf-8"))
        out[(s["setup"]["name"], s["dataset"], s["role"])] = s
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    runs = load()
    if not runs:
        raise SystemExit("no item8-* summaries")
    first = next(iter(runs.values()))
    models = sorted({k[0] for k in runs})
    sets = sorted({k[1] for k in runs})
    L = ["Accuracy of the Hindi and Urdu live (near-live) models (Phase 5, item 8); numbers only; written by eval/item8_report.py.",
         f"machine: {first['machine']['name']} ({first['machine']['cpu']}); engine: {first['engine'].get('version')}; device {first['engine'].get('device')}; "
         f"settings: near-live = greedy, no previous-text context, 15 s chunks; rolling = beam 5 with temperature fallback and previous-text context; "
         "language given (fixed); script-aware normalisation (eval/normalize.py); gate lock held for every run.",
         f"bar (PLAN section 6): near-live WER no more than {BAR_RATIO} times the rolling WER of the ROLLING PICK (the lowest rolling WER of any model run here on the same clips), and RTF at or under {BAR_RTF}. "
         "RTF = harness run time over audio time, model load included, Legion GPU only (it is not a laptop figure).",
         "data: FLEURS hi and ur test (all clips), Kathbath hi and ur test (the Phase 3 lists eval/clips/phase3-kathbath-*.txt, 100 clips each). The English reference is FLEURS-en turbo WER 4.38.",
         "", "model | dataset | clips | near-live WER % | near-live CER % | rolling WER % | rolling CER % | ratio to own rolling | best rolling WER % (model) | ratio to best rolling | near-live RTF | rolling RTF | truncated (near-live) | meets the bar"]
    best = {}
    for (m, ds, role), r in runs.items():
        if role == "rolling" and (ds not in best or r["wer"] < best[ds][0]):
            best[ds] = (r["wer"], m)
    verdict: dict[tuple[str, str], list[bool]] = {}
    for m in models:
        for ds in sets:
            nl, ro = runs.get((m, ds, "near-live")), runs.get((m, ds, "rolling"))
            if not nl and not ro:
                continue
            if not (nl and ro):
                L.append(f"{m} | {ds} | {(nl or ro)['utterances']} | " + (f"{100 * nl['wer']:.1f} | {100 * nl['cer']:.1f} | " if nl else "not run | | ")
                         + (f"{100 * ro['wer']:.1f} | {100 * ro['cer']:.1f}" if ro else "not run | ") + " | | | | | | | incomplete")
                continue
            ratio = nl["wer"] / ro["wer"] if ro["wer"] else float("inf")
            b = best[ds]
            vs_best = nl["wer"] / b[0] if b[0] else float("inf")
            ok = vs_best <= BAR_RATIO and nl["rtf"] <= BAR_RTF
            verdict.setdefault((m, ds.split("-")[1]), []).append(ok)
            L.append(f"{m} | {ds} | {nl['utterances']} | {100 * nl['wer']:.1f} | {100 * nl['cer']:.1f} | {100 * ro['wer']:.1f} | {100 * ro['cer']:.1f} | "
                     f"{ratio:.2f} | {100 * b[0]:.1f} ({b[1]}) | {vs_best:.2f} | {nl['rtf']:.3f} | {ro['rtf']:.3f} | {nl.get('truncated_utterances')} | {'yes' if ok else 'no'}")
    L += ["", f"== Per model and language: meets the bar on every set that was run (WER at most {BAR_RATIO} times the best rolling WER on the same clips, RTF at most {BAR_RTF})",
          "model | language | sets run | meets the bar on all"]
    for (m, lang), oks in sorted(verdict.items()):
        L.append(f"{m} | {lang} | {len(oks)} | {'yes' if all(oks) else 'no'}")
    L += ["", "== Not measured here",
          "first-word latency and the share of segments revised after commit: `wb-engine live` reports speech boundaries only until Phase 7 adds decoding, so "
          "there is nothing to measure; the near-live figures above are batch decodes with the near-live settings.",
          "the room laptops: the RTF bar must be met on the low-baseline laptop's best configuration; this table is the Legion GPU.",
          "`validated_roles` in models/manifest.json is not changed by this report; offering Hindi or Urdu live is the owner's decision (the report only says which rows meet the bar)."]
    text = "\n".join(L) + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
