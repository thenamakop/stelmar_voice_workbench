"""Builds the 5-minute test WAV for the supervisor kill test: FLEURS English test utterances, in id order,
joined with 0.8 s of silence, until at least 300 s. Output is 16 kHz mono 16-bit. Never committed.

    eval/.venv/Scripts/python scripts/gates/make_concat.py --out D:/STELMAR-Gate/killtest/concat-5min.wav
"""

from __future__ import annotations

import argparse
import hashlib
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

from run import load_manifest  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--seconds", type=float, default=300.0)
    ap.add_argument("--dataset", default="fleurs-en")
    args = ap.parse_args()
    utts = sorted(load_manifest(args.dataset), key=lambda u: u.id)
    rate = 16000
    silence = b"\x00\x00" * int(rate * 0.8)
    frames = b""
    used = 0
    for u in utts:
        with wave.open(str(REPO / "eval" / "data" / args.dataset / u.audio)) as w:
            assert w.getframerate() == rate and w.getnchannels() == 1 and w.getsampwidth() == 2
            frames += w.readframes(w.getnframes()) + silence
        used += 1
        if len(frames) / 2 / rate >= args.seconds:
            break
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(out), "wb") as o:
        o.setnchannels(1)
        o.setsampwidth(2)
        o.setframerate(rate)
        o.writeframes(frames)
    print(f"{out.name}: {len(frames) / 2 / rate:.1f} s from {used} {args.dataset} utterances, sha256 {hashlib.sha256(frames).hexdigest()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
