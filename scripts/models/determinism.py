"""Phase 2: quantise (and convert) twice and compare SHA-256, so the manifest can say whether a step is deterministic.

    eval/.venv/Scripts/python scripts/models/determinism.py quantize small medium
    eval/.venv/Scripts/python scripts/models/determinism.py hinglish          # convert twice, then q5_0 twice

Development time only. Each step runs from the same input twice, in two files (`...-a`, `...-b`); the
evidence line says whether the two SHA-256 values are equal. The first file is the one kept (and the one
the manifest pins); when the two differ the manifest says so (`deterministic: false`) and the evidence
says a rerun gives different bytes. Runs at below-normal priority behind the gate lock, like the other
model scripts. Evidence: docs/evidence/phase-2/model-determinism.txt (hashes and names only).
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import logging
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "eval"))

from wbeval import manifest  # noqa: E402
from wbeval.common import (  # noqa: E402
    CONVERTED_DIR, MIN_FREE_GB, MIN_FREE_GB_LARGE, MODELS_DIR, REPO, rel, run_heavy, setup_logging, sha256_file,
)
from wbeval.tools import whisper_cpp_info, whisper_quantize  # noqa: E402

log = logging.getLogger("determinism")
EVIDENCE = REPO / "docs" / "evidence" / "phase-2" / "model-determinism.txt"
TYPES = ("q8_0", "q5_0")
lines: list[str] = []


def say(text: str) -> None:
    log.info(text)
    lines.append(text)


def quantize_once(src: Path, out: Path, qtype: str, large: bool) -> None:
    tmp = out.with_suffix(".tmp")
    with (CONVERTED_DIR / f"{out.stem}.log").open("w", encoding="utf-8") as logf:
        proc = run_heavy([whisper_quantize(), src, tmp, qtype], f"quantising {src.name} to {qtype}",
                         stdout=logf, stderr=logf, min_free_gb=MIN_FREE_GB_LARGE if large else MIN_FREE_GB)
    if proc.returncode != 0:
        tmp.unlink(missing_ok=True)
        raise RuntimeError(f"whisper-quantize failed for {src.name} {qtype}")
    tmp.replace(out)


def quantize_twice(model_id: str, src: Path, base: dict, qtype: str, large: bool) -> dict:
    """Quantises `src` to `qtype` twice; keeps the first output as the model file."""
    final = CONVERTED_DIR / f"ggml-{model_id}-{qtype}.bin"
    again = CONVERTED_DIR / f"ggml-{model_id}-{qtype}-again.bin"
    quantize_once(src, final, qtype, large)
    quantize_once(src, again, qtype, large)
    a, b = sha256_file(final), sha256_file(again)
    again.unlink()
    same = a == b
    wcpp = whisper_cpp_info()
    say(f"quantise {model_id} -> {qtype}: run 1 {a}, run 2 {b}: {'identical (deterministic)' if same else 'DIFFERENT (not deterministic)'}")
    entry = {
        **{k: v for k, v in base.items() if k not in ("id", "file", "sha256", "bytes", "quant", "candidate", "notes",
                                                      "validated_roles", "converted_from_sha256")},
        "id": f"{model_id}-{qtype}", "candidate": base.get("candidate", model_id), "file": f"converted/{final.name}",
        "sha256": a, "bytes": final.stat().st_size, "quant": qtype, "validated_roles": [],
        "converted_from_sha256": sha256_file(src),
        "conversion": {"script": "whisper-quantize", "settings": {"type": qtype, "input": base.get("quant", "f16")},
                       "input_sha256": sha256_file(src), "whisper_cpp_commit": wcpp["commit"],
                       "deterministic": same},
        "notes": f"quantised from {base['id']} ({base.get('quant', 'f16')}) with whisper-quantize {wcpp['tag']}",
    }
    manifest.upsert(entry)
    return entry


def do_quantize(ids: list[str]) -> None:
    CONVERTED_DIR.mkdir(parents=True, exist_ok=True)
    for mid in ids:
        base = manifest.find(mid)
        src = MODELS_DIR / base["file"]
        for qtype in TYPES:
            quantize_twice(mid, src, base, qtype, base["base"].startswith("large"))


def do_hinglish() -> None:
    py = sys.executable
    script = REPO / "scripts" / "models" / "convert_hf_whisper.py"
    out = CONVERTED_DIR / "ggml-hinglish-f16.bin"
    hashes = []
    for n in (1, 2):
        # The converter takes the heavy-step slot (gate lock, memory, busy lock) itself; wrapping it
        # here would make it wait for a lock this process holds.
        t0 = time.perf_counter()
        proc = subprocess.run([py, str(script), "hinglish", "--force"])
        if proc.returncode != 0:
            raise RuntimeError(f"conversion of hinglish failed (run {n})")
        hashes.append(sha256_file(out))
        say(f"convert hinglish -> f16 (vocabulary trimmed to 51866), run {n}: {hashes[-1]} ({time.perf_counter() - t0:.0f} s)")
        if n == 1:
            shutil.copyfile(out, out.with_suffix(".first"))
    same = hashes[0] == hashes[1]
    say(f"convert hinglish: {'identical (deterministic)' if same else 'DIFFERENT (not deterministic)'}")
    first = out.with_suffix(".first")
    if not same:
        # Keep the first output as the file we pin, as the evidence says.
        out.unlink()
        first.replace(out)
        say("the first output is kept and pinned; a rerun gives different bytes")
    else:
        first.unlink()
    record = manifest.find("hinglish-f16")
    wcpp = whisper_cpp_info()
    record["conversion"] = {"script": "scripts/models/convert_hf_whisper.py (whisper.cpp convert-h5-to-ggml.py)",
                            "settings": {"precision": "f16", "truncate_vocab_to": "51866"},
                            "input_sha256": "", "whisper_cpp_commit": wcpp["commit"], "deterministic": same}
    record["sha256"] = hashes[0]
    manifest.upsert(record)
    quantize_twice("hinglish", out, record, "q5_0", True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    q = sub.add_parser("quantize")
    q.add_argument("ids", nargs="+")
    sub.add_parser("hinglish")
    args = parser.parse_args()
    setup_logging("determinism")
    stamp = dt.datetime.now().strftime("%Y-%m-%d %H:%M")
    try:
        if args.cmd == "quantize":
            do_quantize(args.ids)
        else:
            do_hinglish()
    finally:
        if lines:
            EVIDENCE.parent.mkdir(parents=True, exist_ok=True)
            old = EVIDENCE.read_text(encoding="utf-8") if EVIDENCE.exists() else (
                "Model conversion and quantisation determinism (Phase 2)\n"
                "Each step is run twice from the same input; equal SHA-256 means deterministic.\n"
                "whisper-quantize and the converter are from whisper.cpp v1.9.5 (see the manifest).\n\n")
            EVIDENCE.write_text(old + f"-- {stamp} --\n" + "\n".join(lines) + "\n\n", encoding="utf-8")
            print(f"evidence: {rel(EVIDENCE)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
