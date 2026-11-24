"""The tuning set for the language-detection rules (eval/make_tune_set.py): classes come from the reference script, the
held-out clips and the control clips never enter, and the set is the same every time."""

import json
import sys
from pathlib import Path

EVAL = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(EVAL))
import make_tune_set as mts  # noqa: E402


def test_the_class_comes_from_the_script_of_the_reference():
    assert mts.klass("only latin words here") == ("pure-en", 0.0)
    assert mts.klass("यह पूरा वाक्य हिंदी है") == ("pure-hi", 1.0)
    k, share = mts.klass("यह meeting कल है")
    assert k == "mixed" and 0.0 < share < 1.0


def test_the_held_out_and_control_clips_are_never_in_the_set():
    listed = {line.split("\t")[1] for line in (EVAL / "clips" / "phase5-tune-coshe.txt").read_text(encoding="utf-8").splitlines()
              if line and not line.startswith("#")}
    held = {line.split("\t")[1] for line in (EVAL / "clips" / "phase5-coshe-heldout.txt").read_text(encoding="utf-8").splitlines()
            if line and not line.startswith("#")}
    manifest = [json.loads(x) for x in (EVAL / "data" / "coshe-eval" / "manifest.jsonl").read_text(encoding="utf-8").splitlines() if x.strip()]
    controls = {r["id"] for r in manifest[:20]}
    assert listed and not (listed & held) and not (listed & controls)


def test_the_labels_file_holds_ids_and_classes_only():
    d = json.loads((EVAL / "labels" / "coshe-tune.json").read_text(encoding="utf-8"))
    assert {"id", "class", "devanagari_share", "duration_s"} == set(d["clips"][0])
    assert {c["class"] for c in d["clips"]} <= {"mixed", "pure-en", "pure-hi"}
