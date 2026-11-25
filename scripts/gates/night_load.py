"""Phase 5: the night pass's load per conference day from docs/programme.json, and the hours the Hinglish model needs.

    python scripts/gates/night_load.py --out docs/evidence/phase-5/night-load.txt [--factor 1.25]

Inputs: docs/programme.json (the sessions to transcribe, their times and languages) and the committed speed evidence
docs/evidence/phase-5/hinglish-speed-{A,B,C}.txt (Hinglish f16 rolling RTF per language, per device, per profile).
Rules as coded in crates/wb-queue/src/night.rs: the pass starts 30 minutes after the last session to transcribe on the day
ends (the programme overrides the 20:00 evening rule) and must finish by 08:00 IST the next morning; the plan counts the
WHOLE recorded audio of a session at each language's RTF.

The programme lists no language split, only "en, hi" for every session, so the hours are given for a Hindi share of 0,
50 and 100% of the audio (RTF mix = (1 - h) x English RTF + h x Hindi RTF). A session decodes whole, silence included
(the plan counts all of it), so the hours are an upper bound for speech with pauses.
--factor multiplies the RTFs: the detection allowance of a night pass over a session with several languages (the queue's
provisional emu-low x1.25 until a unit of the session has finished).
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
EVID = REPO / "docs" / "evidence" / "phase-5"
START_AFTER_MIN = 30
DEADLINE_H = 8


def hours(t: str) -> float:
    h, m = t.split(":")
    return int(h) + int(m) / 60.0


def speeds() -> dict[str, dict[str, dict[str, float]]]:
    """{tier label: {device label: {lang: rtf}}} for hinglish f16 rolling, from the speed evidence."""
    out: dict[str, dict[str, dict[str, float]]] = {}
    for table, tier in (("A", "emu-low"), ("B", "emu-mid"), ("C", "native RTX 4070")):
        row = None
        for line in (EVID / f"hinglish-speed-{table}.txt").read_text(encoding="utf-8").splitlines():
            if line.startswith("== "):
                row = line[3:].strip()
                continue
            p = [x.strip() for x in line.split("|")]
            if row and len(p) > 5 and p[0] == "hinglish f16" and p[2] == "rolling":
                rtf = float(re.match(r"[0-9.]+", p[4]).group(0))  # type: ignore[union-attr]
                out.setdefault(tier, {}).setdefault(row, {})[p[1]] = rtf
    return out


def machine_days(prog: dict) -> dict[str, dict[str, float]]:
    """Per machine kind and day: (audio hours, end of the last session of the day to transcribe) in hours."""
    days: dict[str, dict[str, dict]] = {}
    for s in prog["sessions"]:
        if not s["transcribe"]:
            continue
        dur = hours(s["end"]) - hours(s["start"])
        d = days.setdefault(s["date"], {"main": 0.0, "hall": 0.0, "all": 0.0, "end": 0.0})
        d["all"] += dur
        d["end"] = max(d["end"], hours(s["end"]))
        d["hall" if s["parallel"] else "main"] += dur
    return days


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out")
    ap.add_argument("--factor", type=float, default=1.25)
    a = ap.parse_args()
    prog = json.loads((REPO / "docs" / "programme.json").read_text(encoding="utf-8"))
    days = machine_days(prog)
    sp = speeds()
    L: list[str] = []
    L.append("Night load per conference day and the hours the Hinglish f16 night pass needs (Phase 5). Written by scripts/gates/night_load.py; nobody edits it.")
    L.append(f"inputs: docs/programme.json ({sum(1 for s in prog['sessions'] if s['transcribe'])} sessions to transcribe, all English + Hindi), docs/evidence/phase-5/hinglish-speed-A/B/C.txt (Legion; emu-low and emu-mid PROVISIONAL)")
    L.append(f"rule as coded (crates/wb-queue/src/night.rs): start {START_AFTER_MIN} min after the last session to transcribe ends; finish by {DEADLINE_H}:00 IST the next morning; the whole recorded audio is decoded")
    L.append(f"RTF factor applied to the benchmark speeds: x{a.factor} (the night detection allowance for a session with several languages, provisional)")
    L.append("")
    L.append("== English + Hindi audio the night pass decodes, per conference day and per kind of machine")
    L.append("machine | date | audio h | last session ends | night starts | hours until 08:00")
    avail: dict[tuple[str, str], tuple[float, float]] = {}
    for kind, label in (("main", "main hall (keynotes, panel, awards: not the parallel tracks)"), ("hall", "seminar hall (parallel tracks only)"), ("all", "one machine for every slot (upper bound: each parallel slot counted once)")):
        for date, d in sorted(days.items()):
            start = d["end"] + START_AFTER_MIN / 60.0
            until = 24 - start + DEADLINE_H
            avail[(kind, date)] = (d[kind], until)
            L.append(f"{label} | {date} | {d[kind]:.2f} | {d['end']:.2f} | {start:.2f} | {until:.1f}")
    L.append("")
    L.append("== Hours of Hinglish f16 per night, by tier and device (Hindi share 0 / 50 / 100% of the audio), against the hours left")
    L.append("tier | device | RTF en | RTF hi | machine | date | audio h | hours at 0% / 50% / 100% Hindi | hours left | finishes by 08:00 (at 50% / at 100% Hindi)")
    verdict: dict[str, list[str]] = {}
    for tier, devs in sp.items():
        for dev, r in devs.items():
            if "near-live" in dev or "en" not in r or "hi" not in r:
                continue
            for (kind, date), (audio, until) in sorted(avail.items()):
                hrs = [audio * a.factor * ((1 - h) * r["en"] + h * r["hi"]) for h in (0.0, 0.5, 1.0)]
                ok50, ok100 = hrs[1] <= until, hrs[2] <= until
                L.append(
                    f"{tier} | {dev} | {r['en']:.3f} | {r['hi']:.3f} | {kind} | {date} | {audio:.2f} | "
                    f"{hrs[0]:.1f} / {hrs[1]:.1f} / {hrs[2]:.1f} | {until:.1f} | {'yes' if ok50 else 'NO'} / {'yes' if ok100 else 'NO'}"
                )
                verdict.setdefault(f"{tier}: {dev}", []).append("yes" if ok100 else ("50% only" if ok50 else "no"))
    L.append("")
    L.append("== Summary: does the Hinglish f16 night pass finish by 08:00 IST on every conference day? (yes = even if all the audio is Hindi; 50% = only if Hindi is half the audio or less; NO = not at 50% Hindi either)")
    for k, res in verdict.items():
        parts = []
        for kind in ("main", "hall", "all"):
            r = [x for (kd, _d), x in zip(sorted(avail), res) if kd == kind]
            parts.append(f"{kind}: " + ("yes" if all(x == "yes" for x in r) else "50%" if all(x in ("yes", "50% only") for x in r) else "NO"))
        L.append(f"{k}: " + "; ".join(parts))
    text = "\n".join(L) + "\n"
    if a.out:
        Path(a.out).parent.mkdir(parents=True, exist_ok=True)
        Path(a.out).write_text(text, encoding="utf-8", newline="\n")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
