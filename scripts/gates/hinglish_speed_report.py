"""Writes the evidence file of scripts/gates/hinglish-speed.ps1 from its saved results (Phase 5, rule (d2)).

    python -X utf8 -I scripts/gates/hinglish_speed_report.py --results D:\\STELMAR-Gate\\speed-hinglish\\results-A.jsonl --table A --machine Legion --out docs/evidence/phase-5/hinglish-speed-A.txt

Numbers only: no text. One block per device row; every cell is judged against the role bars of the system check
(crates/wb-capability/src/rules.rs): rolling RTF <= 1.00, near-live <= 0.35, live stand-in <= 0.25, each multiplied by 0.75
under emulation (LEGION-FIRST section 2: the Legion's efficiency cores and cooling are optimistic for a laptop); the night
pass fits when its RTF times the day's audio (3 h) is within the night window (12 h), the check's defaults, and, for the
programme's last session end of 17:30, the 14.5 h left before 08:00 are shown beside it. Live text needs a discrete GPU, which
an emulated profile does not have: its rows exist only on the native reference table.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

GIB = 1024 ** 3
BARS = {"rolling": 1.0, "near_live": 0.35}
LIVE_STANDIN_BAR = 0.25
HEADROOM = 0.75
DAY_AUDIO_H = 3.0
NIGHT_WINDOW_H = 12.0
UNTIL_0800_H = 14.5  # last session of the programme ends 17:30 (docs/programme.json)
PROFILES = {"A": "emu-low", "B": "emu-mid", "C": "native", "V": "native"}


def gib(v) -> str:
    return "n/a" if v in (None, 0) else f"{v / GIB:.2f}"


def mib(r: dict) -> str:
    """The engine process's graphics memory (DXGI local / shared) in MiB; empty for a processor-only row."""
    local, shared = r.get("gpu_local_bytes"), r.get("gpu_shared_bytes")
    if not local and not shared:
        return ""
    return f"{(local or 0) / 2**20:.0f} / {(shared or 0) / 2**20:.0f}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results", required=True, type=Path)
    ap.add_argument("--table", required=True)
    ap.add_argument("--machine", default="Legion")
    ap.add_argument("--out", required=True, type=Path)
    a = ap.parse_args()
    rows = {}
    for line in a.results.read_text(encoding="utf-8").splitlines():
        if line.strip():
            r = json.loads(line)
            rows[r["key"]] = r
    rows = sorted(rows.values(), key=lambda r: (r["order"], r["model_order"], r["lang"], r["mode"]))
    profile = PROFILES[a.table]
    emulated = profile != "native"
    k = HEADROOM if emulated else 1.0
    first = rows[0]
    L = [f"Hinglish model speed and memory (Phase 5, rule (d2)), table {a.table}: profile {profile}",
         f"date: {first['when']} to {rows[-1]['when']} (IST); machine: {a.machine}; profile: {profile}"
         + (" (wb-emulate: affinity mask and 11 GB job limit, reported as 16 GB; the Intel UHD is the only graphics adapter)" if emulated
            else " (no emulation; reference only, not a tier)"),
         "engines: " + "; ".join(sorted({f"{r['variant']} sha256 {r['engine_sha']}" for r in rows})),
         "whisper.cpp printed by the engine: " + "; ".join(sorted({r['whisper'] for r in rows if r.get('whisper')})),
         f"method: wb-engine bench, the same 60 s clips as the Phase 2 speed tables (en {first['clip_en_s']} s, hi {first['clip_hi_s']} s), "
         "1 warm-up and the median of 3 timed runs (warm-up plus 1 timed run where the warm-up RTF is over 2 under emulation); "
         "rolling = beam 5, temperature fallback, 15 s chunks; near-live = greedy.",
         f"bars: rolling RTF <= {BARS['rolling'] * k:.2f}, near-live <= {BARS['near_live'] * k:.3f}, live stand-in <= {LIVE_STANDIN_BAR * k:.3f}"
         + (" (the 1.00, 0.35 and 0.25 bars times 0.75 headroom)" if emulated else "")
         + f"; night pass fits when RTF x {DAY_AUDIO_H:g} h of audio <= {NIGHT_WINDOW_H:g} h (the system check's rule), i.e. RTF <= "
         f"{NIGHT_WINDOW_H / DAY_AUDIO_H:.2f}; until 08:00 after a 17:30 end there are {UNTIL_0800_H:g} h (RTF <= {UNTIL_0800_H / DAY_AUDIO_H:.2f}).",
         "memory: peak working set and peak commit of the engine process while it ran (an integrated GPU's memory counts as RAM); "
         "under emulation the job limit is 11 GB.", ""]
    row_names = []
    for r in rows:
        if r["row"] not in row_names:
            row_names.append(r["row"])
    best: dict[tuple[str, str], list] = {}
    for name in row_names:
        L.append(f"== {name}")
        L.append("model | lang | role | threads | RTF median (runs) | warm-up RTF | load s | peak working set GiB | peak commit GiB | "
                 "graphics memory MiB (process, local / shared) | "
                 "bar | meets bar | night: hours for 3 h of audio | night fits 12 h | night fits until 08:00")
        for r in [x for x in rows if x["row"] == name]:
            standin = "stand-in" in name
            role = "live stand-in" if standin else ("near-live" if r["mode"] == "near_live" else "rolling")
            bar = (LIVE_STANDIN_BAR if standin else BARS[r["mode"]]) * k
            if r["status"] != "ok":
                L.append(f"{r['model']} | {r['lang']} | {role} | {r['threads']} | {r['status'].upper().replace('_', ' ')} | | | "
                         f"{gib(r.get('peak_ws'))} | {gib(r.get('peak_commit'))} | | {bar:.3f} | no | | | ")
                continue
            rtf = r["rtf_median"]
            runs = ", ".join(f"{x:.3f}" for x in r["rtf_runs"])
            single = " (single timed run)" if r.get("single_run") else ""
            night = ""
            if r["mode"] == "rolling":
                h = rtf * DAY_AUDIO_H
                night = f"{h:.1f} | {'yes' if h <= NIGHT_WINDOW_H else 'no'} | {'yes' if h <= UNTIL_0800_H else 'no'}"
            else:
                night = " | | "
            L.append(f"{r['model']} | {r['lang']} | {role} | {r['threads']} | {rtf:.3f}{single} ({runs}) | {r['warmup_rtf']:.3f} | "
                     f"{r['load_s']:.1f} | {gib(r.get('peak_ws'))} | {gib(r.get('peak_commit'))} | {mib(r)} | {bar:.3f} | "
                     f"{'yes' if rtf <= bar else 'no'} | {night}")
            best.setdefault((r["model"], role), []).append((rtf, name, r["lang"]))
        L.append("")
    L.append("== Summary by model and role (the slowest language of the fastest row; Hindi decides the system check's rule)")
    L.append("model | role | fastest row for the slower language | RTF | meets bar")
    for (model, role), cells in sorted(best.items()):
        by_row: dict[str, float] = {}
        for rtf, name, _lang in cells:
            by_row[name] = max(by_row.get(name, 0.0), rtf)
        name, rtf = min(by_row.items(), key=lambda kv: kv[1])
        bar = (LIVE_STANDIN_BAR if role == "live stand-in" else BARS["near_live" if role == "near-live" else "rolling"]) * k
        L.append(f"{model} | {role} | {name} | {rtf:.3f} | {'yes' if rtf <= bar else 'no'} (bar {bar:.3f})")
    L.append("")
    L.append(f"cells: {len(rows)} (ok {sum(1 for r in rows if r['status'] == 'ok')}, does not fit "
             f"{sum(1 for r in rows if r['status'] == 'does_not_fit')}, other {sum(1 for r in rows if r['status'] not in ('ok', 'does_not_fit'))})")
    if emulated:
        L.append("Emulation is optimistic for CPU speed and pessimistic for integrated-GPU speed (LEGION-FIRST section 2): the CPU rows "
                 "are a ceiling for a real laptop and the UHD rows are a floor.")
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text("\n".join(L) + "\n", encoding="utf-8")
    print(f"written {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
