"""A bilingual test clip from FLEURS: English and Hindi utterances alternate with 0.8 s of silence, until at least --seconds.

    eval/.venv/Scripts/python scripts/gates/make_bilingual.py --out D:/STELMAR-Gate/fit-fallback/bilingual.wav --seconds 120

16 kHz mono 16-bit. Used by scripts/gates/fit-and-fallback.ps1 for the speed and graphics-memory runs that decode a session
with two languages (never committed; no CoSHE audio).
"""

from __future__ import annotations

import argparse
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

from wbeval.data import load_manifest  # noqa: E402

RATE = 16000


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--seconds", type=float, default=120.0)
    a = ap.parse_args()
    en = sorted(load_manifest("fleurs-en"), key=lambda u: u.id)
    hi = sorted(load_manifest("fleurs-hi"), key=lambda u: u.id)
    silence = bytes(2 * int(RATE * 0.8))
    frames = b""
    used = {"en": 0, "hi": 0}
    i = 0
    while len(frames) / 2 / RATE < a.seconds:
        lang, pool = (("en", en), ("hi", hi))[i % 2]
        u = pool[(i // 2) % len(pool)]
        with wave.open(str(REPO / "eval" / "data" / f"fleurs-{lang}" / u.audio)) as w:
            assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2
            frames += w.readframes(w.getnframes()) + silence
        used[lang] += 1
        i += 1
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(out), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(frames)
    print(f"{out}: {len(frames) / 2 / RATE:.1f} s, {used['en']} English and {used['hi']} Hindi utterances")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
