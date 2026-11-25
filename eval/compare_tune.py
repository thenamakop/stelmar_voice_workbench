"""One table of several tuning decodes, side by side (Phase 5, items 4 and 5). Numbers only; no text.

    eval/.venv/Scripts/python -X utf8 -I eval/compare_tune.py <run> [<run> ...] [--out docs/evidence/phase-5/tune-comparison.txt]

Columns: translated chunks (a Hindi-led chunk decoded as English text), chunks whose language matches the label for pure
chunks, English-led mixed chunks and Hindi-led mixed chunks (each on its own), WER, CER, English words written in Devanagari,
the real-time factor and truncated utterances. The labels are derived from the reference text and stay provisional until the
owner's ear check is done (Phase 5, 25 Nov 2026).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from score_tune import score  # noqa: E402


def cell(classes, key: str) -> str:
    c = classes.get(key)
    return f"{c['language matches']}/{c['chunks']}" if c else "-"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="+", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    head = ["run", "models", "rule", "translated", "lang: pure en", "lang: pure hi", "lang: mixed en-led", "lang: mixed hi-led",
            "lang: all", "WER %", "CER %", "English in Devanagari %", "RTF", "truncated"]
    rows = [head]
    for run in a.runs:
        r = score(run)
        s, cl = r["summary"], r["classes"]
        rows.append([r["run"], "+".join(f"{k}={v}" for k, v in sorted(s["setup"]["models"].items())),
                     (s.get("fingerprint_inputs") or {}).get("lang_experiment") or "-", str(r["totals"]["translated"]),
                     cell(cl, "pure English"), cell(cl, "pure Hindi"), cell(cl, "mixed, English-led"), cell(cl, "mixed, Hindi-led"),
                     f"{r['totals']['matches']}/{r['totals']['chunks']}", f"{100 * s['wer']:.1f}", f"{100 * s['cer']:.1f}",
                     f"{100 * s['latin_words']['devanagari_rate']:.1f}", f"{s['rtf']}", str(s.get("truncated_utterances"))])
    first = score(a.runs[0])["summary"]
    L = [f"Tuning-set comparison (Phase 5, items 4 and 5): {first['utterances']} clips; machine {first['machine']['name']}; "
         f"threads {first['settings']['threads']}; device {first['engine'].get('device')}; whisper.cpp {first['engine'].get('version')}.",
         "Chunk-language numbers are PROVISIONAL: the labels come from the reference text and the owner's ear check is not done.", ""]
    L += [" | ".join(r) for r in rows]
    text = "\n".join(L) + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
