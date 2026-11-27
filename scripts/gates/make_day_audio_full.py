"""Builds the audio of the full day simulation, its rehearsal and the engine-kill test (owner, 18 Nov 2026; plan step 7).
Never committed: the output goes under D:\\STELMAR-Gate\\day-sim and holds local evaluation data only.

    eval/.venv/Scripts/python scripts/gates/make_day_audio_full.py --day
    eval/.venv/Scripts/python scripts/gates/make_day_audio_full.py --rehearse
    eval/.venv/Scripts/python scripts/gates/make_day_audio_full.py --kill-test

--day        four sessions of 90 minutes at the programme's times of the first day (10:00, 12:00, 14:30 and 16:00 on the
             scheduler's clock), 48 kHz stereo 16-bit WAV like the sound card the recorder captures from:

               Inaugural   English: Svarah (Indian-accented English) and FLEURS English, 3 to 1
               Parallel I  Hindi: Kathbath Hindi
               Parallel II code-switched Hindi and English: CoSHE-Eval (CC BY-NC 4.0: local only, see below);
                           its clips are 45 to 60 s long, so this session's utterances are long
               Panel       English and Hindi, shaped like a real session (owner, 24 Nov 2026: real panels are English plus
                           Hindi, mostly one speaker plus an emcee): an English introduction (Svarah, first 5% of the
                           session), one speaker alternating English (Svarah) and Hindi (FLEURS Hindi) for the middle, and a
                           short English question-and-answer at the end (last 12%, shorter pauses). A Devanagari session.

--rehearse   the same four sessions of 2 minutes each at compressed times, for the rehearsal that
             runs the whole schedule in under 30 minutes.
--kill-test  two sessions of 30 minutes, one English and Hindi in alternating blocks and one code-switched, for the
             engine-kill test.

Pauses between utterances are 0.5 to 3 s (varied, drawn from a seeded generator) and each session has a few long ones
of 10 to 20 s (4 to 6 s in the rehearsal), so the queue's unit cutter has real pauses to choose from. The audio is
written as it is built (a 90-minute session never sits in memory). Next to the WAV files `day-audio.json` lists every
utterance with its place in the session, its language and its reference text, and the SHA-256 of each file.

CoSHE-Eval is CC BY-NC 4.0: evaluation use, local only. Its audio and its manifest lines stay under D:\\STELMAR-Gate; no
transcript or other data derived from it may be committed, and the evidence may name the set but must not quote it
(`day_report.py` prints numbers only). The manifest marks those utterances `"restricted": "cc-by-nc-4.0"`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import shutil
import sys
import wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from make_day_audio import RATE_IN, RATE_OUT, read_utt, to_48k_stereo  # noqa: E402
from wbeval.data import load_manifest  # noqa: E402

RESTRICTED = {"coshe-eval": "cc-by-nc-4.0"}
MIN_UTT_S, MAX_UTT_S = 1.5, 25.0


class StreamSession:
    """A session written to its WAV file as it is built."""

    def __init__(self, path: Path, title: str, languages: list[str], seed: int, long_pause_s: tuple[float, float], script: str | None = None):
        self.title = title
        self.languages = languages
        self.script = script
        self.path = path
        self.rng = random.Random(seed)
        self.long_pause_s = long_pause_s
        self.refs: list[dict] = []
        self.seconds = 0.0
        self._w = wave.open(str(path), "wb")
        self._w.setnchannels(2)
        self._w.setsampwidth(2)
        self._w.setframerate(RATE_OUT)

    def _write(self, x: np.ndarray) -> None:
        self._w.writeframes(to_48k_stereo(x).tobytes())

    def silence(self, seconds: float) -> None:
        n = int(round(seconds * RATE_IN))
        self._write(np.zeros(n, dtype=np.float32))
        self.seconds += n / RATE_IN

    def add(self, dataset: str, utt, lang: str) -> None:
        x = read_utt(dataset, utt)
        ref = {
            "id": utt.id,
            "dataset": dataset,
            "lang": lang,
            "mixed": bool(getattr(utt, "mixed", False)),
            "start_s": round(self.seconds, 3),
            "end_s": round(self.seconds + len(x) / RATE_IN, 3),
            "text": utt.text,
        }
        if dataset in RESTRICTED:
            ref["restricted"] = RESTRICTED[dataset]
        self.refs.append(ref)
        self._write(x)
        self.seconds += len(x) / RATE_IN

    def pause(self, long: bool = False) -> None:
        lo, hi = self.long_pause_s if long else (0.5, 3.0)
        self.silence(self.rng.uniform(lo, hi))

    def close(self) -> None:
        self._w.close()


def usable(name: str, lo: int = 0, hi: int | None = None, max_s: float = MAX_UTT_S) -> list:
    """The dataset's utterances in id order, within the length limits, sliced; slices of one language never overlap."""
    items = [u for u in sorted(load_manifest(name), key=lambda u: u.id) if MIN_UTT_S <= u.duration_s <= max_s]
    return items[lo:hi]


def fill(s: StreamSession, until_s: float, plan: list, block: dict[str, int], pending_long: list[float]) -> None:
    """Takes `block[dataset]` utterances in turn from each (dataset, iterator, lang) of the plan until the session
    reaches `until_s`. A long pause is inserted at the first utterance end after each time in `pending_long`."""
    while s.seconds < until_s:
        progressed = False
        for dataset, it, lang in plan:
            for _ in range(block.get(dataset, 1)):
                u = next(it, None)
                if u is None:
                    break
                s.add(dataset, u, lang)
                progressed = True
                if pending_long and s.seconds >= pending_long[0]:
                    pending_long.pop(0)
                    s.pause(long=True)
                else:
                    s.pause()
                if s.seconds >= until_s:
                    return
        if not progressed:
            raise SystemExit(f"{s.title}: the datasets ran out of utterances at {s.seconds:.0f} s of {until_s:.0f} s")


def long_pause_times(rng: random.Random, target_s: float, count: int) -> list[float]:
    """Times (within 8% to 92% of the session) after which a long pause falls, spread so no two are close."""
    slots = [(i + rng.uniform(0.15, 0.85)) / count for i in range(count)]
    return sorted(target_s * (0.08 + 0.84 * f) for f in slots)


def build_day_session(out: Path, which: str, target_s: float, rehearse: bool) -> StreamSession:
    seed = {"Inaugural": 31, "Parallel I": 32, "Parallel II": 33, "Panel": 34}[which]
    long_pause = (4.0, 6.0) if rehearse else (10.0, 20.0)
    path = out / (which.lower().replace(" ", "-") + ".wav")
    rng = random.Random(seed * 7)
    count = 1 if rehearse else 5
    pending = long_pause_times(rng, target_s, count)
    if which == "Inaugural":
        s = StreamSession(path, "Inaugural", ["en", "hi"], seed, long_pause)
        plan = [("svarah", iter(usable("svarah", 0, 3000)), "en"), ("fleurs-en", iter(usable("fleurs-en", 0, 300)), "en")]
        block = {"svarah": 3, "fleurs-en": 1}
    elif which == "Parallel I":
        s = StreamSession(path, "Parallel I", ["en", "hi"], seed, long_pause)
        plan = [("kathbath-hi", iter(usable("kathbath-hi", 0, 2500)), "hi")]
        block = {"kathbath-hi": 1}
    elif which == "Parallel II":
        s = StreamSession(path, "Parallel II", ["en", "hi"], seed, long_pause)
        plan = [("coshe-eval", iter(usable("coshe-eval", 0, 400, max_s=61.0)), "hi")]
        block = {"coshe-eval": 1}
    else:
        s = StreamSession(path, "Panel", ["en", "hi"], seed, long_pause)
        intro = iter(usable("svarah", 3000, 3300))
        talk_en = iter(usable("svarah", 3300, 4700))
        talk_hi = iter(usable("fleurs-hi", 0, 400))
        qa = iter(usable("svarah", 4700, 5130))
        block = {"svarah": 4, "fleurs-hi": 4}
        s.silence(1.0)
        # The emcee: English only, the first 5% of the session.
        fill(s, target_s * 0.05, [("svarah", intro, "en")], block, pending)
        # The speaker: English and Hindi in alternating blocks, until the last 12%.
        fill(s, target_s * 0.88, [("svarah", talk_en, "en"), ("fleurs-hi", talk_hi, "hi")], block, pending)
        # The questions and answers: English, short exchanges.
        fill(s, target_s, [("svarah", qa, "en")], {"svarah": 1}, pending)
        s.silence(1.0)
        return s
    s.silence(1.0)
    fill(s, target_s, plan, block, pending)
    s.silence(1.0)
    return s


def build_kill_session(out: Path, which: str, target_s: float) -> StreamSession:
    path = out / (which.lower().replace(" ", "-") + ".wav")
    if which == "Kill English Hindi":
        s = StreamSession(path, which, ["en", "hi"], 41, (10.0, 20.0))
        plan = [("fleurs-en", iter(usable("fleurs-en", 300, 647)), "en"), ("fleurs-hi", iter(usable("fleurs-hi", 0, 418)), "hi")]
        block = {"fleurs-en": 3, "fleurs-hi": 3}
    else:
        s = StreamSession(path, which, ["en", "hi"], 42, (10.0, 20.0))
        plan = [("coshe-eval", iter(usable("coshe-eval", 400, None, max_s=61.0)), "hi")]
        block = {"coshe-eval": 1}
    pending = long_pause_times(random.Random(4242), target_s, 3)
    s.silence(1.0)
    fill(s, target_s, plan, block, pending)
    s.silence(1.0)
    return s


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--day", action="store_true", help="four sessions of 90 minutes at the programme's times")
    g.add_argument("--rehearse", action="store_true", help="the same four sessions of 2 minutes, compressed times")
    g.add_argument("--kill-test", action="store_true", help="two sessions of 30 minutes for the engine-kill test")
    ap.add_argument("--out", default="")
    args = ap.parse_args()
    root = "D:/STELMAR-Gate/day-sim"
    mode = "day" if args.day else ("rehearse" if args.rehearse else "kill-test")
    out = Path(args.out or {"day": f"{root}/day-audio", "rehearse": f"{root}/day-audio-rehearse", "kill-test": f"{root}/kill-audio"}[mode])
    out.mkdir(parents=True, exist_ok=True)
    need_gb = {"day": 4.4, "rehearse": 0.1, "kill-test": 0.7}[mode]
    free_gb = shutil.disk_usage(out).free / 2**30
    if free_gb < need_gb * 2:
        raise SystemExit(f"{out}: {free_gb:.1f} GB free; this needs about {need_gb:.1f} GB and a margin")

    if mode == "kill-test":
        specs = [("Kill English Hindi", None, "Hall A"), ("Kill Code-switched", None, "Hall B")]
        built = [build_kill_session(out, t, 1800.0) for t, _, _ in specs]
        at_s = [None, None]
    else:
        rehearse = mode == "rehearse"
        target = 120.0 if rehearse else 5400.0
        names = ["Inaugural", "Parallel I", "Parallel II", "Panel"]
        built = [build_day_session(out, n, target, rehearse) for n in names]
        # The programme: 10:00, 12:00, 14:30 and 16:00 (seconds after 10:00); compressed for the rehearsal.
        # The rehearsal leaves a long gap before its third session so that the night pass is planned (60 s after the second
        # session's transcript is ready) and then cancelled by that session, which exercises the re-plan (19 Nov 2026).
        at_s = [0, 170, 700, 870] if rehearse else [0, 7200, 16200, 21600]
        specs = [(n, a, h) for n, a, h in zip(names, at_s, ["Main Hall", "Hall A", "Hall B", "Main Hall"])]

    manifest = {"mode": mode, "rate": RATE_OUT, "channels": 2, "restricted": RESTRICTED, "sessions": []}
    for s, (title, at, hall) in zip(built, specs):
        s.close()
        sha = sha256_file(s.path)
        entry = {
            "title": s.title,
            "file": s.path.name,
            "seconds": round(s.seconds, 3),
            "languages": s.languages,
            "hall": hall,
            "sha256": sha,
            "utterances": s.refs,
        }
        if s.script:
            entry["script"] = s.script
        if at is not None:
            entry["at_s"] = at
        manifest["sessions"].append(entry)
        by_set: dict[str, int] = {}
        for r in s.refs:
            by_set[r["dataset"]] = by_set.get(r["dataset"], 0) + 1
        print(f"{s.path.name}: {s.seconds / 60:.1f} min, {len(s.refs)} utterances {by_set}, sha256 {sha}")
    (out / "day-audio.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"manifest: {out / 'day-audio.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
