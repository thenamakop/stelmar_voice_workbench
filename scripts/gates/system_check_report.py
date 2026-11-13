"""Phase 3 system-check gate report: reads the reports the gate hook wrote (WB_GATE_CHECK) and prints the evidence.

    eval/.venv/Scripts/python scripts/gates/system_check_report.py <work-dir> <out.txt>

<work-dir> holds one folder per run, each with report.json (written by the app's gate hook) and data\\capability.json:
    quick-emu-low, quick-emu-mid, quick-native, full-emu-low, view-same-emu-low, view-changed-emu-low
Missing runs are listed as "not run", never guessed.

Per check run: tier, device configuration, time against the quick-check bar, verdict lines, the chosen rolling model per
language with its RTF against the bar, hybrid and recorder-safety consistency, then the measurement table. Then the
quick-versus-full comparison on the same cells (30 s excerpts against 60 s clips; a difference over 10 % is flagged, and
a flag on a cell that decides a model choice, the Hindi turbo cell above all, is marked DECIDING). The evidence holds
numbers and model names only: no audio, no transcript text.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

RUNS = ["quick-emu-low", "quick-emu-mid", "quick-native", "full-emu-low"]
TARGET_S = 360.0  # the quick check aims to end by this (the hard bar is 480 s)
START_BAR_MS = 3000.0  # Start during a check gives a recording within this (the recorder itself starts in well under a second)
FLAG = 0.10
HYBRID_GAIN = 0.10


def stamp(path: Path) -> str:
    """When a report was written: the evidence file is regenerated from every report in the work folder, so each
    section says which invocation it comes from."""
    import datetime

    return datetime.datetime.fromtimestamp(path.stat().st_mtime).strftime("%Y-%m-%d %H:%M") if path.is_file() else "no report"


def load(path: Path):
    if not path.is_file():
        return None
    return json.loads(path.read_text(encoding="utf-8-sig"))


def cfg_key(c: dict) -> str:
    kind = c.get("kind")
    if kind in ("gpu", "hybrid"):
        return f"{kind}:{c.get('index', c.get('gpu'))}"
    return str(kind)


def cell_key(m: dict) -> tuple:
    return (m["model"], m["lang"], m["mode"], cfg_key(m["config"]))


def single_throughput(cap: dict, model: str, lang: str):
    """Best audio-per-second of one device on this model and language (rolling cells)."""
    best = 0.0
    for m in cap["measurements"]:
        if m["model"] == model and m["lang"] == lang and m["mode"] == "rolling" and m["config"].get("kind") != "hybrid":
            best = max(best, 1.0 / m["rtf"])
    return best


def section(label: str, report: dict, lines: list[str], written: str = "") -> dict | None:
    lines.append("")
    lines.append(f"== {label} ==")
    if written:
        lines.append(f"report written: {written}")
    if report is None:
        lines.append("not run")
        return None
    if report.get("error"):
        lines.append(f"FAIL: the check stopped with an error: {report['error']}")
    view = report.get("view") or {}
    cap = view.get("capability")
    if not cap:
        lines.append("FAIL: no capability.json was saved")
        return None
    check, dec = cap["check"], cap["decision"]
    emu = cap.get("emulated")
    wall = report.get("wall_seconds")
    lines.append(f"emulation: {emu or 'none (native)'}; kind {check['kind']}; excerpt {check['excerpt_seconds']:.0f} s; partial {check['partial']}")
    over = check["duration_s"] > check["bar_s"]
    lines.append(
        f"check time: {check['duration_s']:.0f} s measured by the check, {wall:.0f} s wall clock from launch to exit"
        f"; quick-check bar {check['bar_s']:.0f} s ({'OVER the bar' if over else 'within the bar'}"
        f"{'; the bar is for the real low laptop, so this is PROVISIONAL' if emu else ''})"
        if wall is not None
        else f"check time: {check['duration_s']:.0f} s"
    )
    if check["kind"] == "quick":
        lines.append(
            f"quick-check target {TARGET_S:.0f} s: {'met' if check['duration_s'] <= TARGET_S else 'NOT met'} ({check['duration_s']:.0f} s)"
        )
    lines.append(f"tier {dec['tier']}; provisional {dec['provisional']}; partial {dec['partial']}; saved profile fingerprint {view.get('fingerprint_id')}")
    for sk in cap.get("skipped") or []:
        lines.append(f"skipped: {sk['step']}: {sk['reason']}")
    if dec.get("record_only_reason"):
        lines.append(f"record-only reason: {dec['record_only_reason']}")
    if dec.get("config"):
        c = dec["config"]
        lines.append(f"chosen configuration: {c['label']} ({cfg_key(c['config'])}); CPU threads {c.get('cpu_threads')}; recorder safety passed {c.get('recorder_safety_passed')}")
        lines.append(f"  why: {c['why']}")
    for v in dec["verdict"]:
        lines.append(f"  verdict: {v}")
    for lang, ld in dec["languages"].items():
        r = ld.get("rolling")
        if r:
            lines.append(
                f"  {lang}: rolling {r['model']} RTF {r['rtf']:.3f} (bar {dec['bars']['rolling_rtf']:.2f}) meets {r['meets_bar']}; k_max {r['k_max']}; lags {r['lags']}; ready after stop {r['ready_after_stop_min']:.1f} min"
            )
        lv = ld["live"]
        lines.append(f"  {lang}: live offered {lv['offered']}; hard reasons {len(lv['hard_reasons'])}; soft reasons {len(lv['soft_reasons'])}")
    # Hybrid consistency: hybrid chosen only if at least 10 % faster than the best single device and recorder-safe.
    hyb = cap.get("hybrid")
    chosen_hybrid = bool(dec.get("config") and dec["config"]["config"].get("kind") == "hybrid")
    if hyb:
        comb = 1.0 / hyb["gpu_rtf_concurrent"] + 1.0 / hyb["cpu_rtf_concurrent"]
        base = single_throughput(cap, hyb["model"], hyb["lang"])
        gain = comb / base - 1.0 if base else float("nan")
        safe = any(s["config"].get("kind") == "hybrid" and s["passed"] for s in cap["recorder_safety"])
        want = gain >= HYBRID_GAIN and safe
        lines.append(
            f"hybrid test: {hyb['model']} {hyb['lang']}, GPU RTF {hyb['gpu_rtf_concurrent']:.3f} with CPU RTF {hyb['cpu_rtf_concurrent']:.3f} at the same time; combined {comb:.3f} audio/s against best single {base:.3f}: {gain * 100:+.1f}%; recorder safe {safe}"
        )
        lines.append(f"  rule (at least +10% and recorder-safe) says hybrid {want}; the verdict chose hybrid {chosen_hybrid}: {'CONSISTENT' if want == chosen_hybrid else 'INCONSISTENT'}")
    else:
        why = next((s["reason"] for s in cap.get("skipped") or [] if s["step"] == "hybrid test"), None)
        lines.append(
            f"hybrid test: none recorded{' (' + why + ')' if why else ''}; the verdict chose hybrid {chosen_hybrid}: {'CONSISTENT' if not chosen_hybrid else 'INCONSISTENT'}"
        )
    for s in cap["recorder_safety"]:
        lines.append(f"recorder safety {cfg_key(s['config'])}: {s['duration_s']:.0f} s, overruns {s['overruns']}, driver xruns {s['driver_xruns']}, passed {s['passed']}")
    if cap["injectors"]:
        lines.append(f"window-hook injectors found: {', '.join(cap['injectors'])} (the verdict warns; a gate run must not have them)")
    lines.append("measurements (model / language / mode / device / threads: RTF, peak commit, truncated, method):")
    for m in cap["measurements"]:
        lines.append(
            f"  {m['model']:<22} {m['lang']} {m['mode']:<9} {cfg_key(m['config']):<9} {m['threads']:>2} thr  RTF {m['rtf']:.3f}  commit {m['peak_commit_bytes'] / 2**30:.2f} GiB  truncated {m['truncated_chunks']}  {m['method']}"
        )
    return cap


def compare(quick: dict | None, full: dict | None, lines: list[str]):
    lines.append("")
    lines.append("== quick check (short excerpts) against full check (whole bench clips), emu-low ==")
    if not quick or not full:
        lines.append("not compared: one of the two runs is missing")
        return
    qd, fd = quick["decision"], full["decision"]
    deciding = set()
    for lang, ld in qd["languages"].items():
        r = ld.get("rolling")
        if r:
            deciding.add((r["model"], lang))
    for lang, ld in fd["languages"].items():
        r = ld.get("rolling")
        if r:
            deciding.add((r["model"], lang))
    qmap = {cell_key(m): m for m in quick["measurements"]}
    flagged = 0
    deciding_flagged = 0
    compared = 0
    lines.append(f"excerpt {quick['check']['excerpt_seconds']:.0f} s (quick) against {full['check']['excerpt_seconds']:.0f} s (full); flag above {FLAG * 100:.0f}%")
    for m in full["measurements"]:
        q = qmap.get(cell_key(m))
        if not q or m["mode"] != "rolling":
            continue
        compared += 1
        diff = q["rtf"] / m["rtf"] - 1.0
        flag = abs(diff) > FLAG
        dec = (m["model"], m["lang"]) in deciding
        mark = ""
        if flag:
            flagged += 1
            mark = "  FLAG"
            if dec:
                deciding_flagged += 1
                mark += " DECIDING"
        lines.append(f"  {m['model']:<22} {m['lang']} {cfg_key(m['config']):<9} quick {q['rtf']:.3f}  full {m['rtf']:.3f}  {diff * 100:+.1f}%{mark}")
    lines.append(f"cells compared {compared}; flagged {flagged}; flagged and deciding {deciding_flagged}")
    for lang in ("en", "hi"):
        for m in full["measurements"]:
            if m["lang"] == lang and m["mode"] == "rolling" and "turbo" in m["model"] and m["config"].get("kind") == "gpu":
                q = qmap.get(cell_key(m))
                if q:
                    lines.append(
                        f"deciding cell, {lang} turbo on the graphics adapter: quick {q['rtf']:.3f} (30 s excerpt), full {m['rtf']:.3f} (60 s clip): {(q['rtf'] / m['rtf'] - 1.0) * 100:+.1f}%; against the emulated bar 0.75: quick {'meets' if q['rtf'] <= 0.75 else 'misses'}, full {'meets' if m['rtf'] <= 0.75 else 'misses'}"
                    )
    q_hi_turbo = [c for c in quick["measurements"] if c["lang"] == "hi" and "turbo" in c["model"] and c["mode"] == "rolling"]
    if not q_hi_turbo:
        lines.append("Hindi turbo: no rolling cell in the quick run (the ladder stopped before it)")
    lines.append(f"quick versus full verdict: tier {qd['tier']} against {fd['tier']}; " + ("same tier" if qd["tier"] == fd["tier"] else "TIERS DIFFER"))


def start_during_check(report: dict | None, lines: list[str]):
    lines.append("")
    lines.append("== Start pressed during a quick check (emu-low): the check is cancelled, the recording starts ==")
    if report is None:
        lines.append("not run")
        return
    s = report.get("start_during_check") or {}
    if not s:
        lines.append("FAIL: Start was never pressed (the check ended first)")
        return
    ms = s.get("start_took_ms")
    lines.append(
        f"Start pressed {s.get('pressed_after_s', 0):.0f} s into the check (check running at that moment: {s.get('check_was_running')}); the recording was started after {ms:.0f} ms (bar {START_BAR_MS:.0f} ms): "
        f"{'PASS' if s.get('error') is None and ms <= START_BAR_MS else 'FAIL'}; error: {s.get('error')}"
    )
    lines.append(f"check still running right after Start: {s.get('check_running_after_start')} (the command waits up to 5 s for the cancelled check to end)")
    stopped = report.get("stopped") or {}
    lines.append(f"recording stopped by the hook after the check ended: {'yes' if stopped else 'NO'}; recorder summary keys: {sorted(stopped)[:6]}")
    cap = (report.get("view") or {}).get("capability")
    if cap:
        lines.append(
            f"saved profile: partial {cap['check']['partial']} (expected True); measurements {len(cap['measurements'])}; check ran {cap['check']['duration_s']:.0f} s; tier {cap['decision']['tier']}: {'PASS' if cap['check']['partial'] and cap['measurements'] else 'FAIL'}"
        )
    else:
        lines.append("saved profile: none (cancelled before the first measurement; the previous profile stays): acceptable only if no cell had finished")


def fingerprint(same: dict | None, changed: dict | None, lines: list[str]):
    lines.append("")
    lines.append("== fingerprint-change hook (WB_TEST_FINGERPRINT_CHANGE), emu-low ==")
    for label, r, expect in (("unchanged machine", same, False), ("hook set", changed, True)):
        if r is None:
            lines.append(f"{label}: not run")
            continue
        view = r.get("view") or {}
        ch = view.get("fingerprint_changes") or []
        got = len(ch) > 0
        lines.append(f"{label}: {len(ch)} change(s) reported {ch}; expected {'some' if expect else 'none'}: {'PASS' if got == expect else 'FAIL'}")


def main() -> int:
    work = Path(sys.argv[1])
    out = Path(sys.argv[2])
    lines: list[str] = []
    caps = {}
    for name in RUNS:
        rep = load(work / name / "report.json")
        caps[name] = section(name, rep, lines, stamp(work / name / "report.json"))
    compare(caps["quick-emu-low"], caps["full-emu-low"], lines)
    lines.append("")
    lines.append(
        "reports written: " + ", ".join(f"{n} {stamp(work / n / 'report.json')}" for n in ("start-during-check-emu-low", "view-same-emu-low", "view-changed-emu-low"))
    )
    start_during_check(load(work / "start-during-check-emu-low" / "report.json"), lines)
    fingerprint(load(work / "view-same-emu-low" / "report.json"), load(work / "view-changed-emu-low" / "report.json"), lines)
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
