"""Night-pass plan against what the night units did, from the evidence files of day-simulation runs (Phase 5, item 7).
Reads only `evidence.txt` of each run folder (written by day_report.py); no decoding, no models.

    eval/.venv/Scripts/python scripts/gates/night_speed.py D:\\STELMAR-Gate\\day-sim\\day-emu-low-20261121-052323 ^
        D:\\STELMAR-Gate\\day-sim\\smoke5-emu-low-20261123-055853 --out docs/evidence/phase-5/night-speed.txt

Per run: the night plan (hours, the factor it rested on and the units that factor was learned from), the engine seconds
per audio second the rolling units (post-1) and the night units (post-2) actually needed, the night plan's implied
speed, the ratio of actual to planned, and what that ratio means for the largest night the plan would still accept.
The file is evidence (script-written): fix this script and run it again instead of editing the file.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

UNIT = re.compile(r"unit (\d+) of (post-[12]): (\d+) s of audio in (\d+) s on (\w+) -> ([0-9.]+)")
PLAN = re.compile(r"night pass planned for (\d+) session\(s\): about ([0-9.]+) h, ready by (\d\d:\d\d) IST "
                  r"\(measured speed: ([0-9.]+) times the profile's, from (\d+) finished unit")
NIGHT = re.compile(r"night units finished: (\d+); audio ([0-9.]+) of ([0-9.]+) min; engine time ([0-9.]+) min")
ESTIMATE = re.compile(r"the pass would finish at about (\d\d:\d\d:\d\d)")
# The night allowances of crates/wb-queue/src/speed.rs (26 Nov 2026): used before any unit has finished, per device class.
ALLOWANCE = {"graphics adapter": 1.5, "processor": 1.5}
HOURS_TO_DEADLINE = 13.5  # from about 18:30 to 08:00 IST: the room a night plan has on a conference day


def read_run(run: Path) -> dict:
    text = (run / "evidence.txt").read_text(encoding="utf-8", errors="replace")
    units: dict[str, list[tuple[int, int]]] = {"post-1": [], "post-2": []}
    for m in UNIT.finditer(text):
        units[m.group(2)].append((int(m.group(3)), int(m.group(4))))
    plan = PLAN.search(text)
    night = NIGHT.search(text)
    est = ESTIMATE.search(text)
    return {"plan": plan, "night": night, "estimate": est, "units": units}


def per_audio_second(units: list[tuple[int, int]]) -> float | None:
    audio = sum(a for a, _ in units)
    return sum(t for _, t in units) / audio if audio else None


def report(run: Path, P) -> None:
    r = read_run(run)
    P(f"== {run.name}")
    plan, night = r["plan"], r["night"]
    if plan is None:
        P("  no night plan in the evidence")
        return
    hours, ready, factor, learned = float(plan.group(2)), plan.group(3), float(plan.group(4)), int(plan.group(5))
    P(f"  night plan: {plan.group(1)} session(s), about {hours:.1f} h, ready by {ready} IST; it multiplied the profile's night "
      f"speed by {factor:.2f}, learned from {learned} finished unit(s) of the rolling pass")
    roll = per_audio_second(r["units"]["post-1"])
    nite = per_audio_second(r["units"]["post-2"])
    P(f"  rolling units (post-1): {len(r['units']['post-1'])} units, {sum(a for a, _ in r['units']['post-1'])} s of audio, "
      f"{'n/a' if roll is None else f'{roll:.2f}'} engine seconds per audio second")
    P(f"  night units (post-2): {len(r['units']['post-2'])} units, {sum(a for a, _ in r['units']['post-2'])} s of audio, "
      f"{'n/a' if nite is None else f'{nite:.2f}'} engine seconds per audio second")
    if night is None or nite is None:
        P("  no night unit finished: the plan could not be compared with anything (the night pass ended before its first unit)")
        return
    audio_total = float(night.group(3))
    planned_speed = hours * 60.0 / audio_total  # engine minutes per audio minute the plan assumed
    P(f"  night audio {audio_total:.1f} min; the plan assumed {planned_speed:.2f} engine seconds per audio second, the units needed "
      f"{nite:.2f}: actual over planned {nite / planned_speed:.2f}")
    if r["estimate"]:
        P(f"  extrapolated finish of the whole night pass at the speed the units showed: {r['estimate'].group(1)} (planned: {ready})")
    cap_plan = HOURS_TO_DEADLINE * 3600.0 / planned_speed / 60.0
    cap_real = HOURS_TO_DEADLINE * 3600.0 / nite / 60.0
    P(f"  room before 08:00 ({HOURS_TO_DEADLINE:.1f} h): the plan accepts up to {cap_plan:.0f} min of night audio, the units' speed would "
      f"have fitted {cap_real:.0f} min: between those two the plan says it will not finish and the night would have finished")
    # Re-judged with the allowances the queue now uses before any unit has finished (numbers only). The profile's own night
    # speed is the plan's speed divided by the factor it was multiplied by.
    profile_speed = planned_speed / factor
    h, m = ready.split(":")
    start_min = (int(h) * 60 + int(m) - round(hours * 60)) % 1440
    P(f"  re-judged with the night allowances (the profile's night speed here: {profile_speed:.2f} engine seconds per audio second):")
    for name, allowance in ALLOWANCE.items():
        speed = profile_speed * allowance
        needed_h = audio_total * 60.0 * speed / 3600.0
        end = (start_min + round(needed_h * 60)) % 1440
        P(f"    {name}, allowance x{allowance:.2f}: {speed:.2f} engine seconds per audio second, {needed_h:.1f} h for {audio_total:.1f} min of audio, "
          f"ready by {end // 60:02d}:{end % 60:02d} IST; units needed {nite:.2f}: planned over actual {speed / nite:.2f}; room for "
          f"{HOURS_TO_DEADLINE * 3600.0 / speed / 60.0:.0f} min of night audio against {cap_real:.0f}")
    P(f"    as run: x{factor:.2f} (learned from the rolling pass): {hours:.1f} h, ready by {ready} IST; planned over actual {planned_speed / nite:.2f}")
    P(f"  rolling against night speed: {roll:.2f} against {nite:.2f} engine seconds per audio second "
      "(the rolling units ran beside the recorder; in this run the night units ran alone)")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="+", type=Path)
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    lines: list[str] = []
    P = lines.append
    P("Night plan against the night units (Phase 5, item 7). Numbers only; written by scripts/gates/night_speed.py.")
    P("")
    for run in args.runs:
        report(run, P)
        P("")
    text = "\n".join(lines)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
