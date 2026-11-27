"""Hand labels of chunk languages and the score of a run against them (Phase 5, code-switching).

A label file (`eval/labels/<dataset>.json`, committed; time ranges and language tags only, never text)
lists chunks of utterances with the language the speaker led with (`main`) and whether the chunk is
`pure` or `mixed`. A run is scored by finding, for each labelled chunk, the engine segments that overlap
it, taking the language that covers most of the overlap, and comparing it with `main`. This is the same
rule as the Phase 2 gate scorer (docs/evidence/phase-2/codeswitch-labels.txt): a chunk the retry ladder
split into several segments is credited to the language covering most of its time.

The Phase 2 chunks are the held-out final test (`heldout: true` in the file). A tuning script must call
`load_labels(..., heldout=False)`, which refuses to hand them out.

Also reported: chunks with nothing decoded, and chunks that came back translated, meaning the speaker led
in Hindi or Urdu and the text has no letter of that script but three or more Latin words.
"""

from __future__ import annotations

import json
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

from normalize import ARABIC, DEVANAGARI, LATIN, word_script
from wbeval import common

LABELS_DIR = Path(__file__).resolve().parents[1] / "labels"


@dataclass
class ChunkLabel:
    id: str  # utterance id in the dataset
    start_ms: int
    end_ms: int
    kind: str  # pure | mixed
    main: str  # en | hi | ur

    @property
    def seconds(self) -> float:
        return (self.end_ms - self.start_ms) / 1000


class HeldOutError(RuntimeError):
    pass


def load_labels(dataset: str, heldout: bool | None = None, labels_dir: Path | None = None,
                data_dir: Path | None = None) -> list[ChunkLabel]:
    """The labels of a dataset, or an empty list if it has none.

    `heldout=True` returns the held-out final test, `heldout=False` the tuning labels (and raises
    HeldOutError if the only labels there are held out), `None` returns everything (scoring a finished run).
    """
    # Committed labels first (the held-out Phase 2 chunks); then the private labels get_local.py writes next to a local set.
    path = (labels_dir or LABELS_DIR) / f"{dataset}.json"
    if not path.exists():
        path = (data_dir or common.DATA_DIR) / dataset / "labels.json"
    if not path.exists():
        return []
    raw = json.loads(path.read_text(encoding="utf-8"))
    is_heldout = bool(raw.get("heldout", False))
    if heldout is False and is_heldout:
        raise HeldOutError(f"{path.name} is the held-out final test; tune on other data")
    if heldout is True and not is_heldout:
        return []
    return [ChunkLabel(c["id"], int(c["start_ms"]), int(c["end_ms"]), c["kind"], c["main"]) for c in raw["chunks"]]


def _overlap_ms(a0: int, a1: int, b0: int, b1: int) -> int:
    return max(0, min(a1, b1) - max(a0, b0))


def _script_letters(text: str) -> Counter:
    counts: Counter = Counter()
    for word in text.split():
        counts[word_script(word)] += 1
    return counts


def is_translated(main: str, text: str) -> bool:
    """A chunk led in Hindi or Urdu whose text has no word of that script but three or more Latin words."""
    if main not in ("hi", "ur"):
        return False
    counts = _script_letters(text)
    own = counts[DEVANAGARI] if main == "hi" else counts[ARABIC]
    return own == 0 and counts[LATIN] >= 3


def score_chunks(records: list[dict], labels: list[ChunkLabel]) -> dict:
    """Score finished records (run.py `utterances.jsonl` rows) against chunk labels. No text is returned."""
    by_id = {r["id"]: r for r in records}
    rows = []
    confusion: Counter = Counter()
    for lab in labels:
        rec = by_id.get(lab.id)
        if rec is None:
            rows.append({"id": lab.id, "start_ms": lab.start_ms, "end_ms": lab.end_ms, "kind": lab.kind,
                         "main": lab.main, "detected": None, "result": "no record"})
            continue
        time_by_lang: Counter = Counter()
        text = []
        for seg in rec.get("segments", []):
            ov = _overlap_ms(lab.start_ms, lab.end_ms, int(seg["start_ms"]), int(seg["end_ms"]))
            if ov > 0:
                time_by_lang[seg["lang"]] += ov
                text.append(seg.get("text", ""))
        if not time_by_lang:
            rows.append({"id": lab.id, "start_ms": lab.start_ms, "end_ms": lab.end_ms, "kind": lab.kind,
                         "main": lab.main, "detected": None, "result": "nothing decoded"})
            continue
        detected = time_by_lang.most_common(1)[0][0]
        translated = is_translated(lab.main, " ".join(text))
        confusion[(lab.main, detected)] += 1
        rows.append({"id": lab.id, "start_ms": lab.start_ms, "end_ms": lab.end_ms, "kind": lab.kind,
                     "main": lab.main, "detected": detected, "translated": translated,
                     "result": "match" if detected == lab.main else "wrong"})
    scored = [r for r in rows if r["detected"] is not None]
    matches = sum(1 for r in scored if r["result"] == "match")

    def part(kind: str) -> dict:
        sub = [r for r in rows if r["kind"] == kind]
        return {"chunks": len(sub), "matches": sum(1 for r in sub if r["result"] == "match")}

    return {
        "chunks": len(rows),
        "matches": matches,
        "accuracy": matches / len(rows) if rows else None,
        "pure": part("pure"),
        "mixed": part("mixed"),
        "nothing_decoded": sum(1 for r in rows if r["result"] == "nothing decoded"),
        "no_record": sum(1 for r in rows if r["result"] == "no record"),
        "translated": sum(1 for r in scored if r.get("translated")),
        "confusion": {f"{m}->{d}": n for (m, d), n in sorted(confusion.items())},
        "rows": rows,
    }
