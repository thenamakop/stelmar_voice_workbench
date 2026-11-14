"""Evidence report of the Phase 3 turbo-vs-medium comparison (called by turbo-medium-comparison.ps1).

Reads eval/results/phase3-tm-<model>-<set>/summary.json and utterances.jsonl, prints the report
(ASCII only, counts and scores only: no transcript text, the repository is public) and applies the
owner's rule of 13 Nov 2026:

  Hindi and, separately, Urdu: turbo goes first only if its Kathbath WER is no worse than stock
  medium's by more than 2 points; otherwise medium goes first, with the lagging estimate.
  English: FLEURS en settles medium against turbo.

and the owner's two further rules of the same day for large-v3 q5_0 and small:

  Night: large-v3 q5_0 is a language's night model only if its WER (Kathbath for hi and ur, FLEURS for en) is
  at least 2 points better than turbo's; otherwise that language gets no night pass.
  Small fallback: small stays third for a language only if its WER is within 15 points of turbo's;
  otherwise it is removed for that language.

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
MODELS = [
    ("turbo", "large-v3-turbo q5_0"),
    ("medium", "stock medium f16"),
    ("large", "large-v3 q5_0"),
    ("small", "small f16"),
]
SETS = ["fleurs-en", "fleurs-hi", "fleurs-ur", "kathbath-hi", "kathbath-ur"]
RULE_POINTS = 2.0
NIGHT_POINTS = 2.0
SMALL_POINTS = 15.0
LANG_SET = {"en": "fleurs-en", "hi": "kathbath-hi", "ur": "kathbath-ur"}


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
    sha_file = folder / "engine-sha256.txt"
    summary["_engine_sha"] = sha_file.read_text(encoding="utf-8").strip() if sha_file.exists() else "not recorded"
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
    add("Phase 3 informational comparison: large-v3-turbo q5_0, stock medium f16, large-v3 q5_0 and small f16 (NOT a gate, NOT a speed measurement)")
    sample = next((r for r in runs.values() if r), None)
    if sample is None:
        add("no results found under eval/results/phase3-tm-*")
        Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        return 1
    eng = sample["engine"]
    binary = REPO / eng["binary"]
    sha = hashlib.sha256(binary.read_bytes()).hexdigest() if binary.exists() else "binary not found"
    add(f"engine: {eng['version']}")
    add(f"engine file now: {eng['binary']} sha256 {sha}")
    shas = sorted({r["_engine_sha"] for r in runs.values() if r})
    add("engine sha256 of each run is in the last column; builds used: " + ", ".join(x[:16] for x in shas))
    add("the Urdu coverage floor changed from 7.70 to 5.11 on 13 Nov 2026 (docs/evidence/phase-3/kathbath-coverage.txt) and the engine was rebuilt; "
        "every Urdu run and every run of large-v3 q5_0 and small used the rebuilt engine. English and Hindi runs of turbo and medium used the earlier "
        "build, which differs only in the Urdu floor.")
    add(f"device used by all models: {eng.get('device', 'cpu')} {args.device_note}".rstrip())
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
    add("model | set | n | WER % | CER % | WER nukta ignored % | truncated utts | unclear segs | low_coverage segs | retried segs | failures | audio s | wall s | wall RTF | model sha256 | engine sha256")
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
                r["_engine_sha"][:16],
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
    add(f"night rule (owner, 13 Nov 2026): large-v3 q5_0 is a language's night model only if its WER is at least {NIGHT_POINTS:g} points better than "
        "turbo's (Kathbath for hi and ur, FLEURS for en); otherwise that language gets no night pass and the rolling turbo result stands.")
    night = {}
    for lang, ds in LANG_SET.items():
        t, lg = runs[("turbo", ds)], runs[("large", ds)]
        if not t or not lg:
            add(f"{lang}: cannot decide, {ds} is missing")
            night[lang] = "undecided"
            continue
        diff = 100 * (lg["wer"] - t["wer"])
        night[lang] = "large-v3 q5_0" if diff <= -NIGHT_POINTS else "no night pass"
        add(f"{lang}: {ds} WER large-v3 q5_0 {pct(lg['wer'])} against turbo {pct(t['wer'])} (large - turbo = {diff:+.2f} points, "
            f"needs {-NIGHT_POINTS:+g} or lower) -> {night[lang]}")
    add("")
    add(f"small fallback rule (owner, 13 Nov 2026): small stays third for a language only if its WER is within {SMALL_POINTS:g} points of turbo's; "
        "otherwise it is removed for that language (CPU-only weak machines then use turbo on the CPU with the lag estimate).")
    small = {}
    for lang, ds in LANG_SET.items():
        t, sm = runs[("turbo", ds)], runs[("small", ds)]
        if not t or not sm:
            add(f"{lang}: cannot decide, {ds} is missing")
            small[lang] = "undecided"
            continue
        diff = 100 * (sm["wer"] - t["wer"])
        small[lang] = "stays third" if diff <= SMALL_POINTS else "removed"
        add(f"{lang}: {ds} WER small {pct(sm['wer'])} against turbo {pct(t['wer'])} (small - turbo = {diff:+.2f} points, limit {SMALL_POINTS:g}) -> {small[lang]}")
    add("")
    add("result: rolling first: " + ", ".join(f"{k}: {v}" for k, v in verdicts.items())
        + "; night: " + ", ".join(f"{k}: {v}" for k, v in night.items())
        + "; small fallback: " + ", ".join(f"{k}: {v}" for k, v in small.items()) + " (all provisional until Phase 5)")
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
