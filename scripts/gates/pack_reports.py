"""Gate-pack reporters, run by scripts/ingest-gate-results.ps1 on the Legion (the laptops have no Python).

    eval/.venv/Scripts/python -X utf8 scripts/gates/pack_reports.py <work-dir> <out-dir>

<work-dir> is the `work` folder of a results zip (gates/<id>/... as the runner wrote it). It writes, into <out-dir>:

    system-check-quick.txt, system-check-full.txt, quick-vs-full.txt
        the evidence of system_check_report.py (the same section and comparison code the Phase 3 gate used),
        read from gates/quick-check/run/report.json and gates/full-check/run/report.json
    speed-table-igpu.txt, speed-table-cpu.txt
        the speed table recomputed from gates/speed-table-*/results.jsonl: the median of every cell's timed
        runs against the median the laptop recorded, the slowest cells and the adapters with their drivers
    consistency.json
        {"<gate id>": ["problem", ...]} for every gate whose numbers disagree with themselves (a hybrid
        choice the rule does not support, a recorded median that is not the median of its runs, a cell
        with no timed run). The ingest turns a PASS into FAIL and an INFO into INVALID on such a finding;
        it never turns anything into a better status.

No audio and no transcript text is read or written. Missing inputs are listed as "not run", never guessed.
"""

from __future__ import annotations

import json
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import system_check_report as scr  # noqa: E402

REL_TOL = 1e-3


def read_jsonl(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return rows


def check_report(work: Path, gate: str, label: str, out: Path, problems: dict[str, list[str]]):
    report = scr.load(work / "gates" / gate / "run" / "report.json")
    lines: list[str] = []
    cap = scr.section(label, report, lines, scr.stamp(work / "gates" / gate / "run" / "report.json"))
    found = [ln for ln in lines if "INCONSISTENT" in ln or ln.startswith("FAIL:")]
    if found:
        problems.setdefault(gate, []).extend(found)
    (out / f"system-check-{gate.split('-')[0]}.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return cap


def speed_report(work: Path, part: str, out: Path, problems: dict[str, list[str]]):
    gate = f"speed-table-{part}"
    rows = read_jsonl(work / "gates" / gate / "results.jsonl")
    lines = [f"Speed table, {part} part, recomputed from results.jsonl (no audio, no text)"]
    if not rows:
        lines.append("not run: no results.jsonl")
        (out / f"{gate}.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
        return
    bad: list[str] = []
    ok_rows = [r for r in rows if r.get("status") == "ok"]
    lines.append(f"cells: {len(rows)}; ok {len(ok_rows)}; other {len(rows) - len(ok_rows)}")
    devices = sorted({f"{r.get('variant', '?')}: {r.get('device_name')}" for r in ok_rows if r.get("device_name")})
    if devices:
        lines.append("devices named by the engine: " + "; ".join(devices) + " (driver versions are in the evidence of the gate itself)")
    lines.append("model | clip | mode | threads | recorded median | recomputed median | runs | note")
    for r in ok_rows:
        runs = [float(x) for x in (r.get("rtf_runs") or [])]
        rec = r.get("rtf_median")
        note = ""
        if not runs:
            note = "no timed run recorded"
            bad.append(f"{r.get('model')} {r.get('clip')} {r.get('mode')}: ok cell with no timed run")
            again = None
        else:
            again = statistics.median(runs)
            if rec is None or abs(float(rec) - again) > REL_TOL * max(1.0, abs(again)):
                note = "DIFFERENT"
                bad.append(f"{r.get('model')} {r.get('clip')} {r.get('mode')}: recorded median {rec} is not the median {again:.4f} of its runs")
        lines.append(
            f"{r.get('model')} | {r.get('clip')} | {r.get('mode')} | {r.get('threads')} | "
            f"{'n/a' if rec is None else format(float(rec), '.3f')} | {'n/a' if again is None else format(again, '.3f')} | {len(runs)} | {note}"
        )
    slow = sorted(ok_rows, key=lambda r: -float(r.get("rtf_median") or 0.0))[:5]
    lines.append("slowest cells: " + "; ".join(f"{r.get('model')} {r.get('clip')} {float(r.get('rtf_median') or 0):.2f}" for r in slow))
    lines.append(f"consistency: {'PROBLEMS' if bad else 'recorded medians equal the medians of the timed runs'}")
    for b in bad:
        lines.append(f"  problem: {b}")
    if bad:
        problems.setdefault(gate, []).extend(bad)
    (out / f"{gate}.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    work = Path(sys.argv[1])
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    problems: dict[str, list[str]] = {}
    quick = check_report(work, "quick-check", "quick check on this laptop", out, problems)
    full = check_report(work, "full-check", "full check on this laptop", out, problems)
    cmp_lines: list[str] = []
    scr.compare(quick, full, cmp_lines)
    (out / "quick-vs-full.txt").write_text("\n".join(cmp_lines) + "\n", encoding="utf-8")
    for part in ("igpu", "cpu"):
        speed_report(work, part, out, problems)
    (out / "consistency.json").write_text(json.dumps(problems, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {out} ({sum(len(v) for v in problems.values())} problem(s) in {len(problems)} gate(s))")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
