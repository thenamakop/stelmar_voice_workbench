"""How dense the bundled Hindi bench clip is, from its reference text and audio. Numbers only (no text).

    eval/.venv/Scripts/python scripts/gates/hindi_clip_density.py <out.json>

The clip is built like scripts/gates/make_concat.py builds it: FLEURS test utterances in id order joined with
0.8 s of silence. The measure is characters of reference text (without spaces) per second of clip, and per
second of speech (frames above a fixed energy floor). It is compared with every window of consecutive
utterances of the same length (60 s) over the whole FLEURS Hindi test set, so "denser than typical" has a
number: the percentile of the bundled clip among those windows. The same is done for English and Urdu so the
Hindi figure has a reference point. The audio is read from eval/data (never committed); only counts leave.
"""

from __future__ import annotations

import json
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

from run import load_manifest  # noqa: E402

RATE = 16000
GAP_S = 0.8
WINDOW_S = 60.0
FRAME = 480  # 30 ms
FLOOR_DBFS = -45.0


def utter_info(dataset: str):
    rows = []
    for u in sorted(load_manifest(dataset), key=lambda u: u.id):
        with wave.open(str(REPO / "eval" / "data" / dataset / u.audio)) as w:
            n = w.getnframes()
            raw = w.readframes(n)
        samples = [int.from_bytes(raw[i : i + 2], "little", signed=True) for i in range(0, len(raw) - 1, 2)]
        speech_frames = 0
        total_frames = 0
        for i in range(0, len(samples) - FRAME + 1, FRAME):
            chunk = samples[i : i + FRAME]
            rms = (sum(x * x for x in chunk) / FRAME) ** 0.5
            db = 20 * __import__("math").log10(max(rms, 1.0) / 32768.0)
            total_frames += 1
            if db > FLOOR_DBFS:
                speech_frames += 1
        text = getattr(u, "text", None) or getattr(u, "reference", "") or ""
        chars = len("".join(text.split()))
        rows.append(
            {
                "id": u.id,
                "seconds": n / RATE,
                "chars": chars,
                "speech_s": speech_frames * FRAME / RATE,
            }
        )
    return rows


def windows(rows):
    """Every run of consecutive utterances that reaches WINDOW_S the way make_concat does (a window starts at
    each utterance and takes utterances until it holds at least WINDOW_S seconds)."""
    out = []
    for i in range(len(rows)):
        secs = 0.0
        chars = 0
        speech = 0.0
        k = 0
        for r in rows[i:]:
            secs += r["seconds"] + GAP_S
            chars += r["chars"]
            speech += r["speech_s"]
            k += 1
            if secs >= WINDOW_S:
                break
        if secs < WINDOW_S:
            break
        out.append({"first": rows[i]["id"], "utterances": k, "seconds": secs, "cps": chars / secs, "cps_speech": chars / speech if speech else 0.0})
    return out


def percentile_of(value, population):
    below = sum(1 for p in population if p < value)
    return 100.0 * below / len(population)


def main() -> int:
    result = {}
    for dataset, lang in (("fleurs-hi", "hi"), ("fleurs-en", "en"), ("fleurs-ur", "ur")):
        rows = utter_info(dataset)
        wins = windows(rows)
        bundled = wins[0]  # the bundled clip starts at the first utterance in id order
        cps = sorted(w["cps"] for w in wins)
        cpss = sorted(w["cps_speech"] for w in wins)
        result[lang] = {
            "utterances_in_set": len(rows),
            "windows": len(wins),
            "bundled_clip": bundled,
            "chars_per_clip_second": {
                "bundled": bundled["cps"],
                "median": cps[len(cps) // 2],
                "p10": cps[len(cps) // 10],
                "p90": cps[(9 * len(cps)) // 10],
                "percentile_of_bundled": percentile_of(bundled["cps"], cps),
            },
            "chars_per_speech_second": {
                "bundled": bundled["cps_speech"],
                "median": cpss[len(cpss) // 2],
                "p10": cpss[len(cpss) // 10],
                "p90": cpss[(9 * len(cpss)) // 10],
                "percentile_of_bundled": percentile_of(bundled["cps_speech"], cpss),
            },
        }
        # The 30 s excerpt the quick check uses is the first part of the same clip.
        secs = 0.0
        chars = 0
        for r in rows:
            take = min(r["seconds"], 30.0 - secs)
            if take <= 0:
                break
            chars += r["chars"] * (take / r["seconds"])
            secs += take + GAP_S
        result[lang]["first_30s_chars_per_second"] = chars / 30.0
    Path(sys.argv[1]).write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(f"wrote {sys.argv[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
