"""Runs named CoSHE-Eval clips through wb-engine batch with the development attempt log on, to capture the
decode attempts (text and guard metrics) the loop threshold is calibrated on. Output stays in a scratch
folder: it contains transcript text (CoSHE-Eval is CC BY-NC) and is never committed.

    eval/.venv/Scripts/python scripts/gates/capture_attempts.py --out D:/STELMAR-Gate/guard-calibration/attempts.jsonl audio_1444 audio_908 audio_37

Uses the same model (hinglish-q5_0), role (night) and settings as the Hinglish gate run.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

import run as harness  # noqa: E402
from wbeval.engines import WbEngine  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--setup", default="hinglish-q5_0")
    ap.add_argument("--role", default="night")
    ap.add_argument("--dataset", default="coshe-eval")
    ap.add_argument("--no-context", action="store_true", help="decode with previous-text context off")
    ap.add_argument("clips", nargs="+")
    args = ap.parse_args()
    binary = Path(os.environ.get("WB_ENGINE", REPO / "src-tauri" / "binaries" / "wb-engine-avx2-x86_64-pc-windows-msvc.exe"))
    eng = WbEngine(binary=binary)
    setup = harness.parse_setup(args.setup, {})
    settings = harness.role_settings(args.role, 8, None)
    if args.no_context:
        settings.previous_text_context = False
    utts = {u.id: u for u in harness.load_manifest(args.dataset)}
    env = {**os.environ, "WB_ATTEMPT_LOG": args.out}
    for cid in args.clips:
        u = utts[cid]
        wav = REPO / "eval" / "data" / args.dataset / u.audio
        with tempfile.TemporaryDirectory() as t:
            cmd = eng.command(wav, setup, [u.lang], settings, Path(t) / "o.jsonl")
            r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env)
            events = [json.loads(x) for x in r.stdout.splitlines() if x.strip().startswith("{")]
            done = next((e for e in events if e.get("event") == "done"), {})
            warns = [e["code"] for e in events if e.get("event") == "warning"]
            print(cid, "exit", r.returncode, "segments", done.get("segments"), "retried", done.get("retried_chunks"),
                  "unclear", done.get("unclear_chunks"), "warnings", warns, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
