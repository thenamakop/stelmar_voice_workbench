"""Evidence report of the Phase 3 turbo-vs-medium comparison (called by turbo-medium-comparison.ps1).

Reads eval/results/phase3-tm-<model>-<set>/summary.json and utterances.jsonl, prints the report
(ASCII only, counts and scores only: no transcript text, the repository is public) and applies the
owner's rule of 13 Nov 2026:

  Hindi and, separately, Urdu: turbo goes first only if its Kathbath WER is no worse than stock
  medium's by more than 2 points; otherwise medium goes first, with the lagging estimate.
  English: FLEURS en settles medium against turbo.

Usage: eval/.venv/Scripts/python scripts/gates/turbo_medium_report.py OUT.txt [--device-note TEXT]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RESULTS = REPO / "eval" / "results"
MODELS = [("turbo", "large-v3-turbo q5_0"), ("medium", "stock medium f16")]
SETS = ["fleurs-en", "fleurs-hi", "fleurs-ur", "kathbath-hi", "kathbath-ur"]
RULE_POINTS = 2.0


def load(model: str, ds: str) -> dict | None:
    folder = RESULTS / f"phase3-tm-{model}-{ds}"
    if not (folder / "summary.json").exists():
        return None
    summary = json.loads((folder / "summary.json").read_text(encoding="utf-8"))
    segs = []
    for line in (folder / "utterances.jsonl").read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        rec = json.loads(line)
        if "error" in rec:
            continue
        segs.extend(rec.get("segments", []))
    summary["_flags"] = {
        "unclear": sum("unclear" in s.get("flags", []) for s in segs),
        "low_coverage": sum("low_coverage" in s.get("flags", []) for s in segs),
        "retried": sum("retried" in s.get("flags", []) for s in segs),
        "segments": len(segs),
    }
    return summary


def pct(x: float | None) -> str:
    return "n/a" if x is None else f"{100 * x:.2f}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("--device-note", default="")
    args = parser.parse_args()

    runs = {(m, d): load(m, d) for m, _ in MODELS for d in SETS}
    lines: list[str] = []
    add = lines.append
    add("Phase 3 informational comparison: large-v3-turbo q5_0 against stock medium (NOT a gate, NOT a speed measurement)")
    sample = next((r for r in runs.values() if r), None)
    if sample is None:
        add("no results found under eval/results/phase3-tm-*")
        Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        return 1
    eng = sample["engine"]
    binary = REPO / eng["binary"]
    sha = hashlib.sha256(binary.read_bytes()).hexdigest() if binary.exists() else "binary not found"
    add(f"engine: {eng['version']}")
    add(f"engine file: {eng['binary']} sha256 {sha}")
    add(f"device used by both models: {eng.get('device', 'cpu')} {args.device_note}".rstrip())
    add("method: eval/run.py -> wb-engine batch --file per utterance, rolling settings (beam 5, best of 5, temperature fallback, previous-text context, "
        "Silero VAD v6.2.0, 15 s chunks, 8 CPU threads), language given (single-language run), script-aware normaliser (eval/normalize.py); "
        "WER and CER keep the nukta")
    add("sets: FLEURS test, the 20 clips per language of eval/clips/phase2-verify-fleurs.txt; Kathbath test-unknown (tu_), 100 utterances per language, "
        "fixed seed 20261113 (eval/clips/phase3-kathbath-hi.txt, -ur.txt)")
    add("speed note: the RTF column is wall time per audio second including one process start and model load per utterance, on the GPU; it is "
        "NOT comparable with the speed tables (those use one loaded model). Speeds for decisions come from docs/evidence/phase-2/speed-table-A|B|C.txt.")
    add("independence: stock Whisper checkpoints; their training data is not published in detail, so Kathbath and FLEURS are treated as independent "
        "for both models (neither is a fine-tune on these sets). Phase 5 still decides with its own sets.")
    add("")
    add("model | set | n | WER % | CER % | WER nukta ignored % | truncated utts | unclear segs | low_coverage segs | retried segs | failures | audio s | wall s | wall RTF | model sha256")
    for m, label in MODELS:
        for d in SETS:
            r = runs[(m, d)]
            if r is None:
                add(f"{label} | {d} | MISSING")
                continue
            f = r["_flags"]
            msha = next((v["sha256"] for k, v in r["models"].items() if k != "silero-v6.2.0" and v.get("sha256")), "n/a")
            add(" | ".join([
                label, d, str(r["utterances"]), pct(r["wer"]), pct(r["cer"]), pct(r.get("wer_nukta_ignored")),
                str(r["truncated_utterances"]), str(f["unclear"]), str(f["low_coverage"]), str(f["retried"]),
                str(r["failures"]), f"{r['audio_s']:.1f}", f"{r['elapsed_s']:.1f}", f"{r['rtf']:.3f}", msha[:16] + "...",
            ]))
    add("")
    add(f"rule (owner, 13 Nov 2026): turbo first for a language only if its Kathbath WER is no worse than stock medium's by more than {RULE_POINTS:g} points; "
        "English: FLEURS en settles it.")
    verdicts = {}
    for lang, ds in (("hi", "kathbath-hi"), ("ur", "kathbath-ur"), ("en", "fleurs-en")):
        t, md = runs[("turbo", ds)], runs[("medium", ds)]
        if not t or not md:
            add(f"{lang}: cannot decide, {ds} is missing for a model")
            verdicts[lang] = "undecided"
            continue
        diff = 100 * (t["wer"] - md["wer"])
        if lang == "en":
            first = "turbo" if diff < 0 else "medium"
            add(f"en: FLEURS en WER turbo {pct(t['wer'])} against medium {pct(md['wer'])} (turbo - medium = {diff:+.2f} points) -> {first} first "
                "(speeds are equal on the Vulkan iGPU: A/B/C rows 0.354 to 0.374); 20 clips, a difference of this size is within noise unless large")
        else:
            first = "turbo" if diff <= RULE_POINTS else "medium"
            add(f"{lang}: Kathbath WER turbo {pct(t['wer'])} against medium {pct(md['wer'])} (turbo - medium = {diff:+.2f} points, limit {RULE_POINTS:g}) -> "
                f"{first} first" + ("" if first == "turbo" else ", with the lagging estimate"))
        verdicts[lang] = first
        f_ds = ds.replace("kathbath", "fleurs") if lang != "en" else None
        if f_ds and runs[("turbo", f_ds)] and runs[("medium", f_ds)]:
            ft, fm = runs[("turbo", f_ds)], runs[("medium", f_ds)]
            add(f"   for information, FLEURS {lang}: turbo {pct(ft['wer'])} against medium {pct(fm['wer'])} ({100 * (ft['wer'] - fm['wer']):+.2f} points)")
    add("")
    add("result: " + ", ".join(f"{k}: {v}" for k, v in verdicts.items()) + " (all provisional until Phase 5)")
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
