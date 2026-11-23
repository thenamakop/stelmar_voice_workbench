"""Builds the audio the day simulation plays into the virtual cable. Never committed: the output goes under
D:\\STELMAR-Gate\\day-sim (public evaluation data only, see eval/data/SOURCES.md).

    eval/.venv/Scripts/python scripts/gates/make_day_audio.py --smoke

--smoke (the only mode so far): three short sessions for the day-simulation smoke run, 48 kHz stereo 16-bit WAV
like the sound card the recorder captures from:

  Smoke English  about 6.0 min  FLEURS English
  Smoke Hindi    about 6.5 min  FLEURS Hindi
  Smoke Mixed    about 7.5 min  English, Hindi and code-switched (CoSHE-Eval) blocks, so the language changes
                                 from English to Hindi (and back) inside the session

Utterances are joined with pauses of 0.5 to 3 s and, in every session, two long pauses (12 s and 18 s) so the
queue's unit cutter has real pauses to choose from. The sessions use different utterances. Next to each WAV the
manifest `smoke-audio.json` lists every utterance with its place in the session (seconds from the first sample), its
language and its reference text, and the SHA-256 of the file, so the run can be scored later.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
import wave
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

from wbeval.data import load_manifest  # noqa: E402

RATE_IN = 16000
RATE_OUT = 48000


def read_utt(dataset: str, utt) -> np.ndarray:
    with wave.open(str(REPO / "eval" / "data" / dataset / utt.audio)) as w:
        if w.getframerate() != RATE_IN or w.getnchannels() != 1 or w.getsampwidth() != 2:
            raise SystemExit(f"{dataset}/{utt.audio}: expected 16 kHz mono 16-bit")
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32)


def to_48k_stereo(x: np.ndarray) -> np.ndarray:
    n_out = int(round(len(x) * RATE_OUT / RATE_IN))
    t = np.arange(n_out, dtype=np.float64) * (RATE_IN / RATE_OUT)
    y = np.interp(t, np.arange(len(x), dtype=np.float64), x.astype(np.float64))
    y = np.clip(np.round(y), -32768, 32767).astype("<i2")
    return np.repeat(y[:, None], 2, axis=1)


class Session:
    def __init__(self, title: str, languages: list[str], seed: int) -> None:
        self.title = title
        self.languages = languages
        self.rng = random.Random(seed)
        self.parts: list[np.ndarray] = []
        self.refs: list[dict] = []
        self.seconds = 0.0

    def silence(self, seconds: float) -> None:
        self.parts.append(np.zeros(int(round(seconds * RATE_IN)), dtype=np.float32))
        self.seconds += seconds

    def add(self, dataset: str, utt, lang: str) -> None:
        x = read_utt(dataset, utt)
        self.refs.append(
            {
                "id": utt.id,
                "dataset": dataset,
                "lang": lang,
                "mixed": bool(getattr(utt, "mixed", False)),
                "start_s": round(self.seconds, 3),
                "end_s": round(self.seconds + len(x) / RATE_IN, 3),
                "text": utt.text,
            }
        )
        self.parts.append(x)
        self.seconds += len(x) / RATE_IN

    def pause(self, long: bool = False) -> None:
        self.silence(self.rng.choice([12.0, 18.0]) if long else self.rng.uniform(0.5, 3.0))


def build(title: str, languages: list[str], seed: int, target_s: float, plan: list[tuple[str, list]]) -> Session:
    """plan: blocks of (dataset, utterances) taken in turn, one utterance at a time, until the target length."""
    s = Session(title, languages, seed)
    s.silence(1.0)
    iters = [(ds, iter(us), lang) for ds, us, lang in plan]
    long_at = {target_s * 0.33, target_s * 0.66}
    long_done: set[float] = set()
    exhausted = 0
    while s.seconds < target_s and exhausted < len(iters):
        exhausted = 0
        for ds, it, lang in iters:
            n = 0
            for u in it:
                s.add(ds, u, lang)
                n += 1
                due = [t for t in long_at if t not in long_done and s.seconds >= t]
                if due:
                    long_done.add(due[0])
                    s.pause(long=True)
                else:
                    s.pause()
                if n >= BLOCK.get(ds, 1) or s.seconds >= target_s:
                    break
            else:
                exhausted += 1
            if s.seconds >= target_s:
                break
    s.silence(1.0)
    return s


BLOCK: dict[str, int] = {}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--smoke", action="store_true", help="three short sessions for the smoke run")
    ap.add_argument("--long", action="store_true", help="one 14-minute Hindi-heavy session (second smoke: a unit decoded before Stop, a night pass that can finish)")
    ap.add_argument("--minutes", type=float, default=14.0, help="with --long: the session's length in minutes (14 for the second smoke, 22 for the fourth, owner 18 Nov 2026)")
    ap.add_argument("--out", default="")
    args = ap.parse_args()
    if not (args.smoke or args.long):
        print("only --smoke and --long are built so far (the full day's audio is step 7 after the owner's go)", file=sys.stderr)
        return 2
    if not args.out:
        args.out = "D:/STELMAR-Gate/day-sim/smoke-long" if args.long else "D:/STELMAR-Gate/day-sim/smoke"

    en = sorted(load_manifest("fleurs-en"), key=lambda u: u.id)
    hi = sorted(load_manifest("fleurs-hi"), key=lambda u: u.id)
    cs = sorted(load_manifest("coshe-eval"), key=lambda u: u.id)
    cs = [u for u in cs if 3.0 <= u.duration_s <= 20.0]

    sessions: list[Session] = []
    BLOCK.clear()
    if args.long:
        # Four Hindi utterances, then one English, in turn, from the middle of the lists (the first smoke used the
        # ends): about 14 minutes, so the first unit (about 5 minutes) is cut and decoded while the session records.
        BLOCK.update({"fleurs-hi": 4, "fleurs-en": 1})
        sessions.append(
            build("Long Hindi", ["en", "hi"], 21, args.minutes * 60.0, [("fleurs-hi", hi[200:500], "hi"), ("fleurs-en", en[200:500], "en")])
        )
    else:
        # Disjoint slices: English session from the front of the English list, the mixed one from the back, and so on.
        sessions.append(build("Smoke English", ["en", "hi"], 11, 360.0, [("fleurs-en", en[:200], "en")]))
        sessions.append(build("Smoke Hindi", ["en", "hi"], 12, 390.0, [("fleurs-hi", hi[:200], "hi")]))
        # Mixed: a block of 3 English, 3 Hindi, 3 code-switched, in turn, from the other ends of the lists.
        BLOCK.update({"fleurs-en": 3, "fleurs-hi": 3, "coshe-eval": 3})
        sessions.append(
            build(
                "Smoke Mixed",
                ["en", "hi"],
                13,
                450.0,
                [("fleurs-en", reversed(en[-200:]), "en"), ("fleurs-hi", reversed(hi[-200:]), "hi"), ("coshe-eval", cs[:300], "hi")],
            )
        )

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {"rate": RATE_OUT, "channels": 2, "sessions": []}
    for s in sessions:
        pcm = to_48k_stereo(np.concatenate(s.parts))
        name = s.title.lower().replace(" ", "-") + ".wav"
        path = out / name
        with wave.open(str(path), "wb") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(RATE_OUT)
            w.writeframes(pcm.tobytes())
        sha = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest["sessions"].append(
            {
                "title": s.title,
                "file": name,
                "seconds": round(len(pcm) / RATE_OUT, 3),
                "languages": s.languages,
                "sha256": sha,
                "utterances": s.refs,
            }
        )
        print(f"{name}: {len(pcm) / RATE_OUT:.1f} s, {len(s.refs)} utterances, sha256 {sha}")
    (out / "smoke-audio.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"manifest: {out / 'smoke-audio.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
