"""English-only and Urdu sessions: which model for the rolling and night roles (Phase 5, item 10 scoped; night plan of 27 Nov 2026).
Numbers only; no decoding.

    eval/.venv/Scripts/python -X utf8 -I eval/item10_report.py [--out docs/evidence/phase-5/item10-english-urdu.txt]

Reads eval/results/item10-*/summary.json (English: FLEURS-en 200-clip spread, Svarah 150-clip spread; Urdu: large-v3 q5_0 on all of
FLEURS-ur and the Kathbath-ur list) and, for the Urdu comparison, the item 8 rolling runs of the other candidates on the same clips
(eval/results/item8-*-rolling). The rolling and night roles have identical decoding settings in eval/roles.toml, so one WER serves
both; the night pick is the lowest WER and the rolling pick is the lowest WER among models whose speed meets the bar on the
low-baseline laptop (not measurable here: the speed column is the Legion card's RTF and only orders the candidates).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

EVAL = Path(__file__).resolve().parent


def load(pattern: str) -> list[dict]:
    out = []
    for d in sorted((EVAL / "results").glob(pattern)):
        f = d / "summary.json"
        if f.exists():
            out.append(json.loads(f.read_text(encoding="utf-8")))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    eng = load("item10-*")
    urd = [s for s in load("item8-*-rolling") if s["dataset"].endswith("-ur")] + [s for s in eng if s["dataset"].endswith("-ur")]
    if not eng:
        raise SystemExit("no item10-* summaries")
    first = eng[0]
    L = ["English-only and Urdu sessions: rolling and night model choice (Phase 5, item 10 scoped); numbers only; written by eval/item10_report.py.",
         f"machine: {first['machine']['name']} ({first['machine']['cpu']}); engine: {first['engine'].get('version')}; device {first['engine'].get('device')}; "
         "rolling settings (beam 5, temperature fallback, previous-text context, 15 s chunks; identical to the night settings in eval/roles.toml); "
         "language given; script-aware normalisation; gate lock held for every run. RTF = harness run time over audio time on the Legion card (model load included): "
         "it orders the candidates and is not a laptop figure."]

    def table(title: str, rows: list[dict], note: str = "") -> None:
        L.extend(["", f"== {title}", "model | dataset | clips | WER % | CER % | RTF | truncated | broken | wrong script %"])
        for s in rows:
            L.append(f"{s['setup']['name']} | {s['dataset']} | {s['utterances']} | {100 * s['wer']:.1f} | {'n/a' if s['cer'] is None else format(100 * s['cer'], '.1f')} | {s['rtf']:.3f} | "
                     f"{s.get('truncated_utterances')} | {s.get('broken_utf8_utterances')} | {100 * (s.get('wrong_script_rate') or 0):.1f}")
        if note:
            L.append(note)

    def best(rows: list[dict], ds: str) -> str:
        r = sorted((s for s in rows if s["dataset"] == ds), key=lambda s: s["wer"])
        if not r:
            return f"{ds}: no runs"
        second = f"; next {r[1]['setup']['name']} {100 * r[1]['wer']:.1f} ({100 * (r[1]['wer'] - r[0]['wer']):.1f} points behind)" if len(r) > 1 else ""
        return f"{ds}: lowest WER {r[0]['setup']['name']} {100 * r[0]['wer']:.1f}{second}"

    en = [s for s in eng if s["dataset"] in ("fleurs-en", "svarah")]
    table("English-only sessions", sorted(en, key=lambda s: (s["dataset"], s["wer"])))
    L.extend(["", "Lowest WER per set (night pick = lowest on the pooled sets; rolling pick = lowest among those fast enough):",
              best(en, "fleurs-en"), best(en, "svarah")])
    table("Urdu sessions (all candidates on the same clips)", sorted(urd, key=lambda s: (s["dataset"], s["wer"])))
    L.extend(["", "Lowest WER per set:", best(urd, "fleurs-ur"), best(urd, "kathbath-ur")])
    L.extend(["", "== Not decided here",
              "`defaults` and `validated_roles` in models/manifest.json are written by the owner's decision; this report only orders the candidates.",
              "Speed on the low-baseline laptop decides the rolling pick; until the laptops arrive the emu-low speed table (docs/evidence/phase-5/) is the provisional input."])
    text = "\n".join(L) + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
