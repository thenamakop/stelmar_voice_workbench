"""WER per chunk class (eval/class_wer.py): errors are attributed to the chunk of the hypothesis word they align with."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import class_wer  # noqa: E402


def _run(tmp_path: Path, segments: list[dict]) -> Path:
    run = tmp_path / "run"
    run.mkdir()
    (run / "utterances.jsonl").write_text(json.dumps({"id": "u", "segments": segments}) + "\n", encoding="utf-8")
    return run


CHUNKS = [{"id": "u", "start_ms": 0, "end_ms": 5000, "main": "en", "kind": "pure"},
          {"id": "u", "start_ms": 5000, "end_ms": 10000, "main": "hi", "kind": "mixed"}]
REFS = {"u": ("one two three four five six seven eight", 10.0)}


def _by_class(rows):
    out = {}
    for r in rows:
        n, e = out.get(r["cls"], (0, 0))
        out[r["cls"]] = (n + r["n"], e + r["e"])
    return out


def test_a_wrong_word_counts_in_the_chunk_whose_segment_holds_it(tmp_path):
    segs = [{"start_ms": 0, "end_ms": 4000, "text": "one two three four", "lang": "en"},
            {"start_ms": 5000, "end_ms": 9000, "text": "five six seven nine", "lang": "en"}]
    rows = class_wer.score_run(_run(tmp_path, segs), CHUNKS, REFS)
    assert _by_class(rows) == {("en", "pure"): (4, 0), ("hi", "mixed"): (4, 1)}


def test_a_deleted_word_and_an_inserted_word_are_errors_of_a_chunk(tmp_path):
    segs = [{"start_ms": 0, "end_ms": 4000, "text": "one two four", "lang": "en"},
            {"start_ms": 5000, "end_ms": 9000, "text": "five six seven eight extra", "lang": "en"}]
    got = _by_class(class_wer.score_run(_run(tmp_path, segs), CHUNKS, REFS))
    assert got[("en", "pure")] == (4, 1)  # "three" deleted: counted with the next hypothesis word, still in the first chunk
    assert got[("hi", "mixed")] == (4, 1)  # "extra" inserted
