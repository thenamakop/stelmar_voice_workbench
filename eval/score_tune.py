"""Scores a finished decode of the tuning clips against the chunk labels (eval/labels/coshe-tune-chunks.json).

    eval/.venv/Scripts/python -X utf8 -I eval/score_tune.py eval/results/<run> [--out docs/evidence/phase-5/<run>-tune.txt]

Per rule it reports, by chunk class (pure English, pure Hindi, mixed English-led, mixed Hindi-led): the chunks, the chunk
language that matches the label, and the translated chunks (a Hindi-led chunk whose text has three or more Latin words and
no Devanagari word); and over all clips the WER, CER, English words kept in Latin or put in Devanagari, the decode time
and the real-time factor. Numbers only: no text. The tuning labels are never the held-out chunks.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from wbeval.labels import ChunkLabel, score_chunks  # noqa: E402

EVAL = Path(__file__).resolve().parent


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    raw = json.loads((EVAL / "labels" / "coshe-tune-chunks.json").read_text(encoding="utf-8"))
    if raw.get("heldout"):
        raise SystemExit("these are not tuning labels")
    labels = [ChunkLabel(c["id"], c["start_ms"], c["end_ms"], c["kind"], c["main"]) for c in raw["chunks"]]
    records = [json.loads(x) for x in (a.run / "utterances.jsonl").read_text(encoding="utf-8").splitlines() if x.strip()]
    summary = json.loads((a.run / "summary.json").read_text(encoding="utf-8"))
    sc = score_chunks(records, labels)
    classes: dict[str, Counter] = {}
    for row, lab in zip(sc["rows"], labels):
        key = ("pure " if lab.kind == "pure" else "mixed, ") + ("English" if lab.main == "en" else "Hindi") + ("" if lab.kind == "pure" else "-led")
        c = classes.setdefault(key, Counter())
        c["chunks"] += 1
        if row.get("detected") is None:
            c["nothing decoded"] += 1
            continue
        c["language matches"] += row["result"] == "match"
        c["translated"] += bool(row.get("translated"))
    lw = summary["latin_words"]
    L = [f"Tuning-clip score of {a.run.name} (Phase 5, items 4 and 5). Numbers only; written by eval/score_tune.py.",
         f"engine: {summary['engine'].get('version')}; device {summary['engine'].get('device')}; setup {summary['setup']['name']} "
         f"models {summary['setup']['models']}; threads {summary['settings']['threads']}; machine {summary['machine']['name']}; "
         f"languages {summary['expected_langs']}; clips {summary['utterances']}",
         f"labels: {len(labels)} chunks from {raw['from_run']} (derived from the reference script; not hand labels)", "",
         "class | chunks | language matches | translated | nothing decoded"]
    for k in sorted(classes):
        c = classes[k]
        L.append(f"{k} | {c['chunks']} | {c['language matches']} | {c['translated']} | {c['nothing decoded']}")
    L += ["", f"all labelled chunks: {sc['chunks']}; language matches {sc['matches']}; translated {sc['translated']}; "
              f"nothing decoded {sc['nothing_decoded']}",
          f"WER {100 * summary['wer']:.1f}%; CER {100 * summary['cer']:.1f}%; truncated utterances {summary.get('truncated_utterances')}",
          f"English (Latin) reference words {lw['ref_latin_words']}: kept in Latin {lw['kept_latin']}, written in Devanagari "
          f"{lw['in_devanagari']} ({100 * lw['devanagari_rate']:.1f}%), deleted {lw['deleted']}",
          f"decode time {summary['elapsed_s']} s for {summary['audio_s']} s of audio: RTF {summary['rtf']}"]
    text = "\n".join(L) + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
