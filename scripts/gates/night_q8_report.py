"""Night speed of the Hinglish q8_0 model against f16 on a computer without a discrete card (Phase 5, step 3c of the owner's
night plan, 27 Nov 2026). Numbers only; reads the Part N rows of the fit-and-fallback results and docs/programme.json.

    eval/.venv/Scripts/python -X utf8 -I scripts/gates/night_q8_report.py --results D:\\STELMAR-Gate\\fit-fallback\\results.jsonl ^
        --out docs/evidence/phase-5/night-q8.txt

Each row is `wb-engine batch` over a 60 s English + Hindi clip with the model for every chunk (it detects the language of every
chunk itself, as the night job does), default rolling settings, under wb-emulate (emu-low or emu-mid) on the processor (avx2) or
the Intel UHD (Vulkan). RTF = decode seconds over audio seconds (the engine's done event).

Night hours = daily English + Hindi minutes / 60 x RTF x the night allowance (1.5, both device classes; the app multiplies the
speed it plans with by it). The window runs from the last transcribed session of the day plus the 30-minute wait to 08:00 IST the
next day. The programme's English + Hindi minutes per day are those of the sessions marked for transcription with both languages,
one session per time slot (a laptop records one room; parallel sessions run in several halls at once, so a room has one of them).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ALLOWANCE = 1.5
WAIT_MIN = 30
DEADLINE_H = 8
EVENING_FLOOR = None


def hm(s: str) -> int:
    h, m = s.split(":")
    return int(h) * 60 + int(m)


def programme_days(path: Path) -> list[dict]:
    d = json.loads(path.read_text(encoding="utf-8"))
    days: dict[str, dict] = {}
    for s in d["sessions"]:
        if not s.get("transcribe") or not {"en", "hi"} <= set(s.get("languages") or []):
            continue
        day = days.setdefault(s["date"], {"date": s["date"], "day": s["day"], "minutes": 0, "slots": 0, "last_end": 0})
        day["minutes"] += hm(s["end"]) - hm(s["start"])
        day["slots"] += 1
        day["last_end"] = max(day["last_end"], hm(s["end"]))
    out = []
    for day in sorted(days.values(), key=lambda x: x["date"]):
        start = day["last_end"] + WAIT_MIN
        day["window_h"] = (24 * 60 + DEADLINE_H * 60 - start) / 60
        out.append(day)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--results", type=Path, required=True)
    ap.add_argument("--programme", type=Path, default=Path(__file__).resolve().parents[2] / "docs" / "programme.json")
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    rows = [json.loads(l) for l in a.results.read_text(encoding="utf-8").splitlines() if l.strip()]
    rows = [r for r in rows if r.get("part") == "N"]
    days = programme_days(a.programme)
    L = ["Hinglish night model on a computer without a discrete card: q8_0 against f16 (Phase 5, step 3c); numbers only; "
         "written by scripts/gates/night_q8_report.py from the Part N rows of scripts/gates/fit-and-fallback.ps1.",
         f"machine: Legion (RTX 4070 Laptop 8 GB; the emulated profiles use the processor mask and the Intel UHD only); rows run "
         f"{rows[0]['when']} to {rows[-1]['when']} (IST); gate lock held; whisper.cpp as printed by the engines in "
         "docs/evidence/phase-5/engine-builds.txt.",
         "method: wb-engine batch over a 60 s English + Hindi clip (FLEURS utterances alternating, 0.8 s of silence between), the model "
         "for every chunk with detection on, default rolling settings; one run per cell, q8_0 and f16 back to back in the same run of "
         "the script. Emulation is optimistic for the processor and pessimistic for the integrated card (the UHD rows are a floor).",
         "", "== Speed (RTF = decode seconds / audio seconds)",
         "profile | device | threads | model | RTF | peak working set GiB | status | engine sha256 (first 16)"]
    by: dict[tuple, dict] = {}
    for r in rows:
        model = r["key"].split("|")[-1]
        dev = "CPU avx2" if r["key"].split("|")[2] == "cpu" else "Intel UHD (Vulkan)"
        ok = r["status"] == "ok"
        by[(r["profile"], dev, model)] = r
        L.append(f"{r['profile']} | {dev} | {r['threads']} | hinglish-{model} | {r['rtf']:.3f} | {r['peak_ws'] / 2**30:.2f} | ok | {r['engine_sha']}"
                 if ok else f"{r['profile']} | {dev} | {r['threads']} | hinglish-{model} | - | - | {r['status']} | {r['engine_sha']}")
    L += ["", "== q8_0 against f16 (same cell: ratio < 1 means q8_0 is faster)", "profile | device | q8_0 RTF | f16 RTF | q8_0 / f16"]
    for (p, d, m), r in by.items():
        if m != "q8_0":
            continue
        f = by.get((p, d, "f16"))
        if r["status"] == "ok" and f and f["status"] == "ok":
            L.append(f"{p} | {d} | {r['rtf']:.3f} | {f['rtf']:.3f} | {r['rtf'] / f['rtf']:.2f}")
    L += ["", "== The programme's English + Hindi minutes per room and the night window "
          f"(sessions marked for transcription with both languages, one per time slot; window = last end + {WAIT_MIN} min to {DEADLINE_H:02d}:00 IST)",
          "day | date | slots | minutes | last session ends | window hours"]
    for d in days:
        L.append(f"{d['day']} | {d['date']} | {d['slots']} | {d['minutes']} | {d['last_end'] // 60:02d}:{d['last_end'] % 60:02d} | {d['window_h']:.1f}")
    L += ["", f"== Night hours and the 08:00 verdict at the night allowance x{ALLOWANCE} (hours = minutes / 60 x RTF x {ALLOWANCE}; "
          "'plain' = without the allowance)",
          "profile | device | model | day | minutes | hours plain | hours with allowance | window | finishes by 08:00 with allowance | without"]
    fits: dict[tuple, bool] = {}
    for (p, d, m), r in sorted(by.items()):
        if r["status"] != "ok":
            continue
        for day in days:
            plain = day["minutes"] / 60 * r["rtf"]
            allow = plain * ALLOWANCE
            fits[(p, d, m, day["date"])] = allow <= day["window_h"]
            L.append(f"{p} | {d} | hinglish-{m} | {day['day']} | {day['minutes']} | {plain:.1f} | {allow:.1f} | {day['window_h']:.1f} | "
                     f"{'yes' if allow <= day['window_h'] else 'no'} | {'yes' if plain <= day['window_h'] else 'no'}")
    typical = 180
    L += ["", f"== Room minutes of English + Hindi that finish by 08:00 at the allowance x{ALLOWANCE} (window hours x 60 / (RTF x {ALLOWANCE})); "
          f"the typical day of the system check is {typical} min (3 h)",
          "profile | device | model | after a 17:30 end (window 14.0 h) | after a 16:30 end (window 15.0 h) | 180 min finishes"]
    for (p, d, m), r in sorted(by.items()):
        if r["status"] != "ok":
            continue
        cap = [w * 60 / (r["rtf"] * ALLOWANCE) for w in (days[0]["window_h"], days[-1]["window_h"])]
        L.append(f"{p} | {d} | hinglish-{m} | {cap[0]:.0f} min | {cap[1]:.0f} min | {'yes' if typical / 60 * r['rtf'] * ALLOWANCE <= days[0]['window_h'] else 'no'}")
    L += ["", "== Does q8_0 finish wherever f16 does?"]
    worse = []
    for (p, d, m, date), ok in sorted(fits.items()):
        if m == "q8_0" and fits.get((p, d, "f16", date)) and not ok:
            worse.append((p, d, date))
    L.append("yes: every cell in which f16 finishes by 08:00 with the allowance, q8_0 finishes too." if not worse
             else "NO: f16 finishes and q8_0 does not in: " + "; ".join(f"{p}, {d}, {date}" for p, d, date in worse))
    gained = sorted({(p, d, date) for (p, d, m, date), ok in fits.items() if m == "q8_0" and ok and not fits.get((p, d, "f16", date))})
    L.append("cells where q8_0 finishes and f16 does not: " + ("; ".join(f"{p}, {d}, {date}" for p, d, date in gained) if gained else "none"))
    text = "\n".join(L) + "\n"
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
