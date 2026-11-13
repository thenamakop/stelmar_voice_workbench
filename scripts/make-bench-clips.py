"""Builds the clips the system check times: `resources/bench/bench-{en,hi,ur}.wav` and
`bench-codeswitch.wav`, each exactly 60 s, 16 kHz mono 16-bit, from FLEURS test utterances (CC BY 4.0).

    eval/.venv/Scripts/python scripts/make-bench-clips.py

Each language clip joins that language's FLEURS test utterances in id order with 0.8 s of silence and is cut
at 60 s (the last sentence may end early; speed does not depend on it). `bench-codeswitch.wav` alternates
English and Hindi utterances the same way, so every switch is at a sentence boundary. It stands in for a
recording of real code-switched speech until Phase 4's local test set exists; the System screen and
`capability.json` call it a stand-in. `resources/bench/clips.json` lists the utterance ids and the SHA-256 of
each file, so the clips can be rebuilt and checked. The script is deterministic.
"""

from __future__ import annotations

import hashlib
import json
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "eval"))

from run import load_manifest  # noqa: E402

RATE = 16000
SECONDS = 60.0
GAP = b"\x00\x00" * int(RATE * 0.8)
OUT = REPO / "resources" / "bench"


def pcm(dataset: str, audio: str) -> bytes:
    with wave.open(str(REPO / "eval" / "data" / dataset / audio)) as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2, (dataset, audio)
        return w.readframes(w.getnframes())


def join(items: list[tuple[str, str]]) -> tuple[bytes, list[str]]:
    """(dataset, audio) pairs joined until SECONDS are reached, cut at SECONDS."""
    want = int(RATE * SECONDS) * 2
    data = b""
    used: list[str] = []
    for dataset, audio in items:
        data += pcm(dataset, audio) + GAP
        used.append(f"{dataset}/{audio}")
        if len(data) >= want:
            break
    if len(data) < want:
        raise SystemExit(f"not enough audio for {SECONDS} s ({len(data) / 2 / RATE:.1f} s)")
    return data[:want], used


def write(name: str, data: bytes) -> str:
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / name
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(data)
    return hashlib.sha256(path.read_bytes()).hexdigest()


def utterances(dataset: str) -> list[tuple[str, str]]:
    return [(dataset, u.audio) for u in sorted(load_manifest(dataset), key=lambda u: u.id)]


def main() -> int:
    index: dict[str, dict] = {}
    for lang in ("en", "hi", "ur"):
        data, used = join(utterances(f"fleurs-{lang}"))
        index[f"bench-{lang}.wav"] = {"sha256": write(f"bench-{lang}.wav", data), "utterances": used, "source": f"FLEURS test {lang}"}
    en, hi = utterances("fleurs-en"), utterances("fleurs-hi")
    alternating: list[tuple[str, str]] = []
    for i in range(min(len(en), len(hi))):
        alternating.append(en[i] if i % 2 == 0 else hi[i])
    data, used = join(alternating)
    index["bench-codeswitch.wav"] = {
        "sha256": write("bench-codeswitch.wav", data),
        "utterances": used,
        "source": "FLEURS test en and hi, alternating sentences (stand-in for real code-switched speech)",
    }
    (OUT / "clips.json").write_text(json.dumps({"seconds": SECONDS, "rate": RATE, "gap_s": 0.8, "clips": index}, indent=2) + "\n", encoding="utf-8")
    for name, meta in index.items():
        print(f"{name}  {meta['sha256'][:12]}  {len(meta['utterances'])} utterances")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
