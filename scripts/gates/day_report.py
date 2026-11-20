"""Writes the evidence text of a full day simulation, its rehearsal or the engine-kill test from the run folder.
Nothing is edited by hand: fix this script and run it again (AGENTS.md, evidence files).

    eval/.venv/Scripts/python scripts/gates/day_report.py <run folder> <out.txt> --mode day|rehearse|kill

Reads what day-sim.ps1 left in the run folder: report.json (the app's report), queue.log (the scheduler's log, scheduler
clock first), script-notes.json, day-plan.json, idle-check.json, fidelity.json and fidelity-N.txt (the Phase 1 recording
rule, per session), process-samples.csv, and for every session folder under rec/ the segment files, transcript.db and the
work folder. It also writes timeline.svg next to them (plain SVG, no library).

CoSHE-Eval (CC BY-NC 4.0) is used as local evaluation data only. This report names it and prints numbers about it
(counts, error rates, durations); it never prints a reference or hypothesis text of any session.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from collections import defaultdict
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from day_sim_report import find_sessions, jload, jsonl, seg_checks, sql  # noqa: E402

EVAL = Path(__file__).resolve().parents[2] / "eval"
BAR_MEM = 0.70
START_CALL_BAR_MS = 1000  # owner, 20 Nov 2026: the start call returns in under 1 s under load
NIGHT_DEADLINE_H = 8.0  # 08:00 the next morning, as hours after 00:00 of the first day: 24 + 8
NIGHT_DEADLINE_CLOCK = 24.0 + NIGHT_DEADLINE_H


def hms(x: float) -> str:
    x = x % 86400
    return f"{int(x // 3600):02d}:{int(x % 3600 // 60):02d}:{int(x % 60):02d}"


class Clock:
    """The scheduler's 'HH:MM:SS' column as seconds since 00:00 of day one (keeps counting past midnight)."""

    def __init__(self, first_hour: float = 10.0):
        self.last = first_hour * 3600.0
        self.day = 0

    def parse(self, text: str) -> float:
        parts = [int(t) for t in text.split(":")]
        h, m = parts[0], parts[1]
        sec = parts[2] if len(parts) > 2 else 0  # the scheduler log has minutes only
        v = h * 3600 + m * 60 + sec + self.day * 86400
        while v < self.last - 43200:  # a time far before the last one: the clock passed midnight
            self.day += 1
            v += 86400
        self.last = max(self.last, v)
        return v


def read_qlog(run: Path) -> list[tuple[float, str]]:
    p = run / "queue.log"
    if not p.exists():
        return []
    clock = Clock()
    out = []
    for raw in p.read_text(encoding="utf-8", errors="replace").splitlines():
        m = re.match(r"^(\d{1,2}:\d{2}(?::\d{2})?)\t(.*)$", raw)
        if m:
            out.append((clock.parse(m.group(1)), m.group(2)))
    return out


def engine_deaths(qlog) -> int:
    """How many times the supervisor logged an engine death ('unit N of post-M job J on cpu: Died {...}'). The
    '(N restart(s))' text of a unit's 'done' line is not used for this: it does not count every death (in the kill
    test of 20 Nov 2026 a unit that died five times says 1)."""
    return sum(1 for _t, x in qlog if re.search(r": unit \d+ of post-\d+ job \d+ on \w+: Died \{", x))


def stop_order_of(qlog: list[tuple[float, str]]) -> dict[str, int]:
    """Session folder name -> index in the scheduler log of its `DAY: stopped` line (the app's own stop of the session)."""
    folder_of: dict[str, str] = {}
    order: dict[str, int] = {}
    for k, (_t, text) in enumerate(qlog):
        m = re.match(r'^DAY: started "(.+?)" in (.+)$', text)
        if m:
            folder_of[m.group(1)] = Path(m.group(2).strip().replace("\\", "/")).name
            continue
        m = re.match(r'^DAY: stopped "(.+?)"', text)
        if m and m.group(1) in folder_of:
            order[folder_of[m.group(1)]] = k
    return order


def outstanding_at_stop(
    stop_order: dict[str, int],
    line_no: int,
    name: str,
    candidates: list[str],
    complete_at: dict[tuple[str, str], float],
    t: float,
) -> list[str]:
    """The sessions whose rolling transcript was outstanding at the Stop of `name`, whose 'ready by' is the log line
    `line_no`: that session itself and every other session that had already stopped (its `DAY: stopped` line comes
    earlier in the log) and whose rolling transcript was not complete yet. A session that had not stopped is not
    outstanding, whatever its time: the scheduler log has a resolution of one minute, and Panel, started the minute
    Parallel II stopped, was counted as outstanding at that Stop (day simulation of 19-20 Nov 2026)."""
    out = [
        n
        for n in candidates
        if n != name and stop_order.get(n, 1 << 60) < line_no and complete_at.get((n, "post-1"), 1e12) > t
    ]
    out.append(name)
    return out


def ready_by(text: str, after: float) -> float | None:
    """'ready by HH:MM IST' as the first such clock time after `after` (seconds since 00:00 of day one)."""
    m = re.search(r"ready by (\d{1,2}):(\d{2}) IST", text)
    if not m:
        return None
    v = int(m.group(1)) * 3600 + int(m.group(2)) * 60
    while v < after - 60:
        v += 86400
    return float(v)


def pct(a: float, b: float) -> str:
    return f"{100.0 * a / b:.1f}%" if b else "n/a"


def wer_numbers(mode_sessions: list[dict], run: Path, audio_dir: Path | None, P) -> None:
    """Word and character error rates against the references, for information only (never a gate)."""
    if audio_dir is None or not (audio_dir / "day-audio.json").exists():
        P("  no day-audio.json: error rates not computed")
        return
    sys.path.insert(0, str(EVAL))
    try:
        import metrics  # type: ignore
    except Exception as e:  # jiwer missing in this interpreter, or the eval folder moved
        P(f"  error rates not computed: {type(e).__name__}: {e}")
        return
    manifest = json.loads((audio_dir / "day-audio.json").read_text(encoding="utf-8-sig"))
    refs_by_title = {s["title"]: s for s in manifest["sessions"]}
    P("  per session, whole text: reference words, WER, CER (the final revision: post-2 where complete, else post-1)")
    per_set: dict[str, list] = defaultdict(list)
    for s in mode_sessions:
        ref_s = refs_by_title.get(s["title"])
        if not ref_s:
            continue
        rev = "post-2" if s["post2_complete"] else "post-1"
        segs = sorted(jsonl(s["dir"] / f"{rev}.jsonl"), key=lambda x: x["start_ms"])
        if not segs:
            P(f"  {s['title']}: no segments in {rev}")
            continue
        ref_text = " ".join(u["text"] for u in ref_s["utterances"])
        hyp_text = " ".join(x["text"] for x in segs)
        sc = [metrics.prepare(s["title"], ref_text, hyp_text, "mix", True)]
        w, c = metrics.wer(sc), metrics.cer(sc)
        P(f"  {s['title']} ({rev}): reference words {len(sc[0].ref.split())}, hypothesis words {len(sc[0].hyp.split())}, WER {'n/a' if w is None else f'{100 * w:.1f}%'}, CER {'n/a' if c is None else f'{100 * c:.1f}%'}")
        # Per dataset: each utterance's window, shifted by where the source content starts in the app's recording.
        # Where the source starts in the app's recording: where it starts in the reference capture (speech_analyse.py) plus
        # the one fixed offset between the app and the reference (compare.json: an app frame is a reference frame plus it).
        ra = jload(run / f"fid-{s['index']}" / "reference-analysis.json") if s.get("index") else None
        cmp_ = jload(run / f"fid-{s['index']}" / "compare.json") if s.get("index") else None
        segs0 = (cmp_ or {}).get("segments") or []
        if ra is None or not segs0 or segs0[0].get("offset_frames") is None:
            continue
        shift = float(ra["content_start_ms"]) + float(segs0[0]["offset_frames"]) * 1000.0 / 48000.0
        for u in ref_s["utterances"]:
            lo, hi = u["start_s"] * 1000.0 + shift, u["end_s"] * 1000.0 + shift
            hyp = " ".join(x["text"] for x in segs if lo <= (x["start_ms"] + x["end_ms"]) / 2.0 <= hi)
            per_set[u["dataset"]].append(metrics.prepare(u["id"], u["text"], hyp, u["lang"], bool(u.get("mixed"))))
    if per_set:
        P("  per dataset, utterance by utterance, each utterance's window from where the source starts in the app's recording (reference capture start plus the fixed offset), information:")
        for name in sorted(per_set):
            items = per_set[name]
            w, c = metrics.wer(items), metrics.cer(items)
            tag = " (CC BY-NC 4.0, local evaluation data: numbers only)" if name == "coshe-eval" else ""
            P(f"    {name}{tag}: {len(items)} utterances, WER {'n/a' if w is None else f'{100 * w:.1f}%'}, CER {'n/a' if c is None else f'{100 * c:.1f}%'}")


def timeline_svg(path: Path, rows: list[dict], marks: list[tuple[float, str]], t0: float, t1: float) -> None:
    """A plain SVG: one lane per session (recording, rolling units, night units), a lane for night, vertical marks."""
    W, LH, LEFT, TOP = 1400, 26, 190, 40
    lanes = []
    for r in rows:
        lanes.append((r["label"], r))
    height = TOP + LH * (len(lanes) + 1) + 60
    span = max(1.0, t1 - t0)

    def x(t: float) -> float:
        return LEFT + (W - LEFT - 20) * (t - t0) / span

    colour = {"record": "#2a6f4e", "post-1": "#2b5fa8", "post-2": "#8a4fa0", "estimate": "#c0861a"}
    o = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{height}" font-family="sans-serif" font-size="11">']
    o.append(f'<rect width="{W}" height="{height}" fill="#ffffff"/>')
    o.append(f'<text x="10" y="20" font-size="13" font-weight="bold">Day simulation timeline (scheduler clock, IST)</text>')
    h = int(t0 // 3600)
    while h * 3600 <= t1 + 1:
        gx = x(h * 3600)
        if gx >= LEFT:
            o.append(f'<line x1="{gx:.1f}" y1="{TOP - 6}" x2="{gx:.1f}" y2="{height - 50}" stroke="#dddddd"/>')
            o.append(f'<text x="{gx:.1f}" y="{TOP - 10}" text-anchor="middle" fill="#555555">{h % 24:02d}:00</text>')
        h += 1
    for i, (label, r) in enumerate(lanes):
        y = TOP + LH * i
        o.append(f'<text x="8" y="{y + 16}" fill="#222222">{label}</text>')
        for kind, a, b in r["bars"]:
            o.append(f'<rect x="{x(a):.1f}" y="{y + 4}" width="{max(1.5, x(b) - x(a)):.1f}" height="{LH - 8}" fill="{colour.get(kind, "#888888")}" opacity="0.85"><title>{kind} {hms(a)} to {hms(b)}</title></rect>')
        for t, txt in r.get("ticks", []):
            o.append(f'<path d="M{x(t):.1f},{y + 2} l4,-0 l-4,{LH - 4} l-4,-{LH - 4} z" fill="{colour["estimate"]}"><title>{txt}</title></path>')
    for t, txt in marks:
        o.append(f'<line x1="{x(t):.1f}" y1="{TOP}" x2="{x(t):.1f}" y2="{height - 50}" stroke="#b03030" stroke-dasharray="4 3"/>')
        o.append(f'<text x="{x(t) + 3:.1f}" y="{height - 38}" fill="#b03030">{txt}</text>')
    ly = height - 20
    for k, (name, c) in enumerate([("recording", colour["record"]), ("rolling unit (post-1)", colour["post-1"]), ("night unit (post-2)", colour["post-2"]), ("ready-by estimate at Stop", colour["estimate"])]):
        o.append(f'<rect x="{10 + 210 * k}" y="{ly - 9}" width="12" height="10" fill="{c}"/><text x="{26 + 210 * k}" y="{ly}" fill="#222222">{name}</text>')
    o.append("</svg>")
    path.write_text("\n".join(o), encoding="utf-8")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("out")
    ap.add_argument("--mode", choices=["day", "rehearse", "kill"], required=True)
    a = ap.parse_args()
    run, out, mode = Path(a.run), Path(a.out), a.mode
    L: list[str] = []
    P = L.append

    report = jload(run / "report.json") or {}
    notes = jload(run / "script-notes.json") or {}
    plan = jload(run / "day-plan.json") or {}
    idle = jload(run / "idle-check.json") or {}
    fid = {f["title"]: f for f in (jload(run / "fidelity.json") or {}).get("sessions", [])}
    q = report.get("queue", {})
    app_log = q.get("log", [])
    qlog = read_qlog(run)
    audio_dir = Path(notes["audio_dir"]) if notes.get("audio_dir") else None
    pn = run / "partial-note.txt"
    partial = pn.read_text(encoding="utf-8-sig").strip() if pn.exists() else ""

    title = {"day": "FULL DAY SIMULATION", "rehearse": "DAY-SIMULATION REHEARSAL (not a gate: the whole schedule compressed, to prove the script end to end)", "kill": "ENGINE-KILL TEST"}[mode]
    if partial:
        P("DAY-SIMULATION PARTIAL RUN 1: INVALID as a day simulation, kept as information")
        for pl in partial.splitlines():
            P("  " + pl)
        P("")
    P(title + (" (PROVISIONAL: emulation profile, the real laptops repeat it)" if mode != "rehearse" else ""))
    P(f"run folder: {run.name}")
    P(f"profile in force: {notes.get('profile')}")
    P(f"app exit code {notes.get('emu_exit')}; scheduler clock offset {notes.get('clock_offset_s')} s (the first session of the plan maps to 10:00 IST); parts of {notes.get('part_seconds')} s; night time box {notes.get('night_minutes')} min")
    P(f"quiet-machine rule: {notes.get('quiet')} (idle-check.json: {len(idle.get('attempts', []))} attempt(s), waited {idle.get('waited_min')} min; the last: host CPU {(idle.get('attempts') or [{}])[-1].get('cpu_avg')}% over {idle.get('seconds')} s, busy {(idle.get('attempts') or [{}])[-1].get('busy')}, the owner's programs running {(idle.get('attempts') or [{}])[-1].get('owner_programs')})")
    P(f"display drivers: at the start {notes.get('drivers_start')}; at the end {notes.get('drivers_end')}")
    if notes.get("drivers_changed"):
        P(f"RUN INVALID: {notes.get('driver_change')} (a profile is only valid for the driver it was made with; the run must be repeated)")
    P(f"configuration the queue worked in: {q.get('configuration')}; forced hybrid: {report.get('forced_hybrid')}")
    P("")
    P("Script notes (progress):")
    for l in notes.get("progress", []):
        P(f"  {l}")
    P("")

    # ---------------------------------------------------------------- sessions
    by_dir = {Path(s["dir"]).name: s for s in report.get("sessions", [])}
    folders = find_sessions(run / "rec")
    sess: list[dict] = []
    for d in folders:
        meta = jload(d / "session.json") or {}
        parts = (jload(d / "parts.json") or {}).get("parts", [])
        rep = by_dir.get(d.name) or {}
        db = d / "transcript.db"
        revs = {r[0]: r for r in sql(db, "SELECT name, complete, is_default FROM revisions")}
        sess.append(
            {
                "name": d.name,
                "dir": d,
                "title": meta.get("title", rep.get("title", d.name)),
                "imported": d.name.endswith("-copy") or (not rep and not partial),
                "audio_ms": sum(p.get("duration_ms", 0) for p in parts),
                "parts": len(parts),
                "report": rep,
                "post1_complete": bool(revs.get("post-1") and revs["post-1"][1]),
                "post2_complete": bool(revs.get("post-2") and revs["post-2"][1]),
                "default": next((r[0] for r in revs.values() if r[2]), None),
                "index": None,
                "opened": meta.get("opened_at", ""),
            }
        )
    recorded = [s for s in sess if not s["imported"]]
    for i, r in enumerate(sorted(recorded, key=lambda s: s["report"].get("started_ist") or s.get("opened", "")), start=1):
        r["index"] = i
    names = {s["name"]: s for s in sess}

    P("Sessions under rec/:")
    for s in sess:
        P(f"  {s['name']}: title {s['title']!r}, {'imported copy' if s['imported'] else 'recorded'}, {s['parts']} parts, {s['audio_ms'] / 60000.0:.1f} min of audio, post-1 complete {s['post1_complete']}, post-2 complete {s['post2_complete']}, default revision {s['default']}")
    P("")

    checks: list[tuple[str, bool | None, str]] = []  # (name, passed or None for information, detail)

    # ---------------------------------------------------------------- the log lines the checks rest on
    unit_done = []  # (clock, session folder, unit idx, revision, took s, device, restarts, peak MiB, limit MiB, deaths)
    for t, text in qlog:
        m = re.match(r"^(\S+): unit (\d+) of (post-\d+) done in ([\d.]+) s on (\w+) \((\d+) restart\(s\), engine peak (\d+) MiB against limit (\d+) MiB(?:, (\d+) memory-limit death)?", text)
        if m:
            unit_done.append((t, m.group(1), int(m.group(2)), m.group(3), float(m.group(4)), m.group(5), int(m.group(6)), int(m.group(7)), int(m.group(8)), int(m.group(9) or 0)))
    complete_at: dict[tuple[str, str], float] = {}
    for t, text in qlog:
        m = re.match(r"^(\S+): (post-\d) transcript complete", text)
        if m:
            complete_at[(m.group(1), m.group(2))] = t
    day_start: dict[str, float] = {}
    day_stop: dict[str, float] = {}
    for t, text in qlog:
        m = re.match(r'^DAY: (started|stopped) "(.+?)"', text)
        if m:
            (day_start if m.group(1) == "started" else day_stop)[m.group(2)] = t
    title_of = {s["name"]: s["title"] for s in sess}

    # ---------------------------------------------------------------- (1) fidelity and recorder counters
    if mode in ("day", "rehearse"):
        P("RECORDING FIDELITY  each session against its own independent reference capture of the cable (the Phase 1 rule, docs/PLAN.md section 12)")
        for s in sorted(recorded, key=lambda s: s["index"] or 0):
            f = fid.get(s["title"], {})
            P(f"  {s['title']}: {f.get('verdict', 'NO VERDICT')}; {f.get('audio_minutes')} min; upstream breaks {f.get('upstream_breaks')}, counted {f.get('counted_breaks')} of at most {f.get('break_limit')}; differing frames {f.get('differing_frames')}; runs of the app {f.get('runs_of_the_app')}; dropped samples {f.get('dropped_samples')}, ring overrun samples {f.get('overrun_samples')}, xruns {f.get('xruns')}")
            for r in ([f["invalid_reasons"]] if isinstance(f.get("invalid_reasons"), str) else (f.get("invalid_reasons") or [])):
                P(f"      INVALID: {r}")
            for r in ([f["fail_reasons"]] if isinstance(f.get("fail_reasons"), str) else (f.get("fail_reasons") or [])):
                P(f"      FAIL: {r}")
            ftxt = run / f"fidelity-{s['index']}.txt"
            if ftxt.exists():
                for line in ftxt.read_text(encoding="utf-8", errors="replace").splitlines():
                    P(f"      {line}")
            verdict = f.get("verdict", "NO VERDICT")
            checks.append((f"fidelity of '{s['title']}' (one offset per run, 0 dropped, upstream breaks within max(1, floor(minutes/10)))", True if verdict == "PASS" else (None if verdict == "INVALID" else False), f"{verdict}" + (" (INVALID: the session must be repeated; no verdict)" if verdict == "INVALID" else "")))
            clean = all(int(f.get(k) or 0) == 0 for k in ("dropped_samples", "overrun_samples", "xruns"))
            checks.append((f"recorder counters of '{s['title']}': 0 ring overruns, 0 xruns, 0 dropped samples", clean, f"dropped {f.get('dropped_samples')}, overrun {f.get('overrun_samples')}, xruns {f.get('xruns')}"))
        P("")

    # ---------------------------------------------------------------- (2) rolling estimates against what happened
    P("ROLLING ESTIMATES  the 'ready by' recorded at each Stop against the moment everything that was outstanding at that Stop had finished")
    stop_order = stop_order_of(qlog)
    est_rows = []
    for k, (t, text) in enumerate(qlog):
        m = re.match(r"^(\S+): stopped\. All transcripts ready by (\d{1,2}:\d{2}) IST", text)
        if not m:
            continue
        name = m.group(1)
        rb = ready_by(text, t)
        # Only the sessions that had already stopped at this Stop can be outstanding (by the order of the log lines,
        # not by their one-minute times; a session that starts after this Stop has nothing to finish yet).
        outstanding = outstanding_at_stop(stop_order, k, name, [n for n, s in names.items() if not s["imported"]], complete_at, t)
        done = [complete_at.get((n, "post-1")) for n in outstanding]
        finished = max(done) if all(d is not None for d in done) else None
        basis = re.search(r"\((.*)\)\s*$", text)
        est_rows.append((name, t, rb, finished, outstanding, basis.group(1)[:70] if basis else ""))
    for name, t, rb, finished, outstanding, basis in est_rows:
        if rb is None:
            P(f"  {title_of.get(name, name)}: Stop {hms(t)}; no ready-by in the log line")
            continue
        est = max(60.0, rb - t)
        if finished is None:
            P(f"  {title_of.get(name, name)}: Stop {hms(t)}, estimate ready by {hms(rb)} ({est / 60:.1f} min); the outstanding transcripts did not all finish")
            checks.append((f"rolling estimate at Stop of '{title_of.get(name, name)}' within 10%", False, "not everything outstanding at Stop finished"))
            continue
        act = finished - t
        err = (act - est) / est
        P(f"  {title_of.get(name, name)}: Stop {hms(t)}, estimate {est / 60:.1f} min (ready by {hms(rb)}), actual {act / 60:.1f} min (finished {hms(finished)}), difference {100 * err:+.1f}%, {len(outstanding)} transcript(s) outstanding; basis: {basis}")
        checks.append((f"rolling transcripts outstanding at the Stop of '{title_of.get(name, name)}' finished within 10% of the estimate recorded then" + (" (information in the rehearsal: sessions of 2 minutes)" if mode == "rehearse" else ""), (abs(err) <= 0.10) if mode != "rehearse" else None, f"estimate {est / 60:.1f} min, actual {act / 60:.1f} min ({100 * err:+.1f}%)"))
    if not est_rows:
        P("  no 'stopped. All transcripts ready by' line in the log")
    P("")

    # ---------------------------------------------------------------- (2b) the start call and how each session ended
    P("START CALLS  how long the app's start call took for each session, step by step (the recorder had begun when it returned)")
    start_total: dict[str, int] = {}
    for _t, text in qlog:
        m = re.match(r"^START: (start call of (.+?): .*total (\d+) ms)$", text)
        if m:
            start_total[Path(m.group(2).replace("\\", "/")).name] = int(m.group(3))
            P(f"  {m.group(1)}")
        elif text.startswith("START: "):
            P(f"    {text[7:]}")
    if not start_total:
        P("  no start-call timing in the log (an older gate build)")
    over = {n: ms for n, ms in start_total.items() if ms > START_CALL_BAR_MS}
    if start_total:
        worst = max(start_total.values())
        checks.append((
            f"the start call of every session returned within {START_CALL_BAR_MS} ms of the recorder starting (worst {worst} ms)",
            not over,
            ", ".join(f"{n} {ms} ms" for n, ms in over.items()),
        ))
    timer_stops = [x for _t, x in qlog if "was stopped by the safety timer" in x]
    for x in timer_stops:
        P(f"  {x}")
    checks.append((
        "every session was stopped by its stop file, 15 s after its player finished (no safety timer)",
        not timer_stops,
        "; ".join(timer_stops),
    ))
    P("")

    # ---------------------------------------------------------------- (3) hybrid only where the profile chose it
    P("DEVICES  hybrid only where the profile chose it")
    cfg = (notes.get("profile") or {}).get("config", "")
    chose_hybrid = "ybrid" in str(cfg)
    devs = defaultdict(int)
    for u in unit_done:
        devs[(u[3], u[5])] += 1
    P(f"  profile config: {cfg}; forced hybrid (labelled, kill test only): {report.get('forced_hybrid')}")
    P("  units done by revision and slot: " + ", ".join(f"{k[0]} on {k[1]}: {v}" for k, v in sorted(devs.items())))
    slots = {k[1] for k in devs}
    if mode == "kill":
        P(f"  both slots used (forced hybrid, labelled): {sorted(slots)}")
    else:
        checks.append(("hybrid only where the profile chose it", chose_hybrid or len(slots) <= 1, f"slots used {sorted(slots)}, profile hybrid {chose_hybrid}"))
    P("")

    # ---------------------------------------------------------------- (4) memory
    P("ENGINE MEMORY  peak against the limit the queue granted (bar: 70%)")
    worst, worst_row = 0.0, None
    deaths = 0
    for u in unit_done:
        r = u[7] / max(1, u[8])
        deaths += u[9]
        if r > worst:
            worst, worst_row = r, u
    if worst_row:
        P(f"  worst: {worst * 100:.1f}% ({worst_row[7]} of {worst_row[8]} MiB; {worst_row[1]} unit {worst_row[2]} of {worst_row[3]} on {worst_row[5]})")
    P(f"  memory-limit deaths (reported apart from other failures): {deaths}")
    checks.append(("engine peak memory under 70% of its limit in every unit", worst <= BAR_MEM and bool(unit_done), f"worst {worst * 100:.1f}%"))
    if mode != "kill":
        checks.append(("no engine died at its memory limit", deaths == 0, f"{deaths}"))
    P("")

    # ---------------------------------------------------------------- (5) the transcripts
    P("TRANSCRIPT INTEGRITY  (single writer: no overlap, no repeated start, audio order, cursor)")
    integrity_ok = True
    for s in sess:
        for rev in ("post-1", "post-2"):
            segs = jsonl(s["dir"] / f"{rev}.jsonl")
            if not segs:
                continue
            c = seg_checks(segs)
            ordered = sorted(segs, key=lambda x: x["start_ms"])
            # Commit order: the order they were appended in the file is the order they were committed in.
            in_order = all(segs[i]["start_ms"] <= segs[i + 1]["start_ms"] for i in range(len(segs) - 1))
            cur = sql(s["dir"] / "transcript.db", "SELECT cursor_ms FROM jobs WHERE revision=? ORDER BY id DESC LIMIT 1", (rev,))
            cursor = cur[0][0] if cur else None
            last_end = max(x["end_ms"] for x in segs)
            cursor_ok = cursor is not None and cursor >= last_end - 1
            ok = c["overlaps"] == 0 and c["duplicate_starts"] == 0 and in_order and cursor_ok
            integrity_ok &= ok
            speech = sum(x["end_ms"] - x["start_ms"] for x in segs)
            P(f"  {s['name']} {rev}: {c['count']} segments, overlaps {c['overlaps']}, duplicate starts {c['duplicate_starts']}, appended in audio order {in_order}, job cursor {cursor} ms vs last segment end {last_end} ms, speech {speech / 1000.0:.0f} s of {s['audio_ms'] / 1000.0:.0f} s, last end {last_end / 1000.0:.0f} s -> {'ok' if ok else 'PROBLEM'}")
    checks.append(("no segment repeated or overlapping, appended in audio order, cursor at the end, in every revision", integrity_ok, ""))
    for s in recorded:
        checks.append((f"rolling transcript of '{s['title']}' complete and its revision the default (until the night pass)", s["post1_complete"], f"post-1 complete {s['post1_complete']}, default {s['default']}"))
    P("")

    # ---------------------------------------------------------------- (6) the night pass
    P("NIGHT PASS")
    night_lines = [(t, x) for t, x in qlog if "night" in x.lower() or "post-2" in x]
    cancelled_lines = [(t, x) for t, x in qlog if "night pass cancelled" in x]
    # The last plan counts: a plan made before a later session started is cancelled and made again (19 Nov 2026).
    planned = next(((t, x) for t, x in reversed(qlog) if "night pass planned" in x), None)
    skipped = next(((t, x) for t, x in qlog if "night pass skipped" in x), None)
    n_complete = {n: t for (n, rv), t in complete_at.items() if rv == "post-2"}
    if planned:
        pt, px = planned
        n_complete = {n: t for n, t in n_complete.items() if t >= pt}
        if cancelled_lines:
            P(f"  {len(cancelled_lines)} earlier plan(s) cancelled because a session started after it was planned and planned again after the last session:")
            for ct, cx in cancelled_lines:
                P(f"    {hms(ct)} {cx}")
        mm = re.search(r"for (\d+) session", px)
        expected = int(mm.group(1)) if mm else 0
        rb = ready_by(px, pt)
        P(f"  planned at {hms(pt)}: {px}")
        night_units = [u for u in unit_done if u[3] == "post-2"]
        # Audio of the night jobs, from the transcript databases.
        total_audio = 0.0
        done_audio = 0.0
        for s in sess:
            for j in sql(s["dir"] / "transcript.db", "SELECT id FROM jobs WHERE kind='night'"):
                for u in sql(s["dir"] / "transcript.db", "SELECT start_ms, end_ms, state FROM units WHERE job=?", (j[0],)):
                    d = (u[1] - u[0]) / 1000.0
                    total_audio += d
                    if u[2] in ("done", "committed", "complete"):
                        done_audio += d
        took = sum(u[4] for u in night_units)
        last_t = max([u[0] for u in night_units], default=pt)
        P(f"  night units finished: {len(night_units)}; audio {done_audio / 60.0:.1f} of {total_audio / 60.0:.1f} min; engine time {took / 60.0:.1f} min; sessions with a complete post-2: {len(n_complete)} of {expected}")
        if rb is not None:
            P(f"  planned 'ready by' {hms(rb)}")
        if expected and len(n_complete) >= expected:
            fin = max(n_complete.values())
            P(f"  night pass finished at {hms(fin)} (the pass took {(fin - pt) / 60.0:.1f} min from its planning) -> {'before 08:00' if fin <= NIGHT_DEADLINE_CLOCK * 3600 else 'AFTER 08:00'}")
            checks.append(("night pass finished before 08:00 IST", fin <= NIGHT_DEADLINE_CLOCK * 3600, f"finished {hms(fin)}"))
        else:
            rate = done_audio / took if took > 0 else 0.0
            remaining = max(0.0, total_audio - done_audio)
            if rate > 0:
                est_fin = last_t + remaining / rate
                P(f"  ESTIMATE (extrapolated; the time box ended the run): {done_audio / 60:.1f} of {total_audio / 60:.1f} min of night audio done at {rate:.3f} audio seconds per engine second; the remaining {remaining / 60:.1f} min take about {remaining / rate / 60:.0f} min more, so the pass would finish at about {hms(est_fin)} -> {'before 08:00' if est_fin <= NIGHT_DEADLINE_CLOCK * 3600 else 'AFTER 08:00'}")
                checks.append(("night pass: extrapolated finish before 08:00 IST (ESTIMATE; the time box ended the run)", est_fin <= NIGHT_DEADLINE_CLOCK * 3600, f"estimated {hms(est_fin)}"))
            else:
                P("  no night unit finished: nothing to extrapolate from")
                checks.append(("night pass: extrapolated finish before 08:00 IST", False, "no night unit finished"))
    elif skipped:
        P(f"  skipped at {hms(skipped[0])}: {skipped[1]}")
        checks.append(("night pass correctly skipped (the reason is in the log)", True, skipped[1][:120]))
    else:
        P("  no night decision in the log")
        checks.append(("night pass planned or skipped", mode == "kill", "no decision"))
    for t, x in night_lines:
        if "planned" not in x and "skipped" not in x:
            P(f"  {hms(t)} {x}")
    P("")

    # ---------------------------------------------------------------- (7) kills
    if mode == "kill":
        P("ENGINE KILLS  the script kills batch engines from outside (taskkill /F); the supervisor restarts them")
        ks = notes.get("kills") or []
        by_kind: dict[str, int] = defaultdict(int)
        for k in ks:
            by_kind[k["Kind"]] += 1
            P(f"  kill {k['N']}: {k['Kind']} engine pid {k['Pid']} after {k['AgeS']} s at {k['At']}, taskkill exit {k['Exit']}")
        P(f"  kills made: {len(ks)} ({dict(by_kind)}); target 5 of each kind")
        restarts = engine_deaths(qlog)
        P(f"  engine deaths the supervisor logged (each is followed by a restart of the unit): {restarts}")
        done_restarts = sum(u[6] for u in unit_done)
        P(f"  (the unit-done lines report {done_restarts} restart(s): they count since a unit's last death, not all of them)")
        for t, x in qlog:
            if re.search(r"failed on the|failed three times|failed on every|parked|memory-limit death", x):
                P(f"  {hms(t)} {x}")
        checks.append(("5 CPU (avx2) engines killed", by_kind.get("avx2", 0) >= 5, f"{by_kind.get('avx2', 0)}"))
        checks.append(("5 GPU (vulkan) engines killed", by_kind.get("vulkan", 0) >= 5, f"{by_kind.get('vulkan', 0)}"))
        checks.append(("every kill was followed by a restart or a retry of the unit (engine deaths the supervisor logged at least the kills made)", restarts >= len(ks), f"{restarts} engine deaths logged for {len(ks)} kills"))
        P("")
        P("  Each recorded session against its imported copy decoded by the same queue (the copies are decoded without a kill only when the kills are over; the report states the counts):")
        for s in recorded:
            cp = next((c for c in sess if c["imported"] and c["title"] == s["title"]), None)
            if not cp:
                P(f"    {s['title']}: no imported copy")
                continue
            a_segs = sorted(jsonl(s["dir"] / "post-1.jsonl"), key=lambda x: x["start_ms"])
            b_segs = sorted(jsonl(cp["dir"] / "post-1.jsonl"), key=lambda x: x["start_ms"])
            ra = {(x["start_ms"], x["end_ms"]) for x in a_segs}
            rb2 = {(x["start_ms"], x["end_ms"]) for x in b_segs}
            same = len(ra & rb2)
            import difflib

            ta = " ".join(x["text"] for x in a_segs)
            tb = " ".join(x["text"] for x in b_segs)
            sim = difflib.SequenceMatcher(None, ta, tb, autojunk=False).ratio() if ta and tb else 0.0
            sa = sum(x["end_ms"] - x["start_ms"] for x in a_segs) / 1000.0
            sb = sum(x["end_ms"] - x["start_ms"] for x in b_segs) / 1000.0
            P(f"    {s['title']}: killed run {len(a_segs)} segments ({sa:.0f} s of speech), copy {len(b_segs)} segments ({sb:.0f} s); identical time ranges {same} of {len(ra)}; text similarity {sim:.3f} (information: text after a restart may differ in the first chunk)")
            checks.append((f"'{s['title']}': speech covered in the killed run within 3% of the uninterrupted copy", abs(sa - sb) <= 0.03 * max(sa, sb, 1.0), f"{sa:.0f} s against {sb:.0f} s"))
        P("")

    # ---------------------------------------------------------------- (8) error rates, who used the machine
    if mode != "kill":
        P("ERROR RATES  information only (not a gate); reference text of every utterance is from the local evaluation sets")
        for s in sess:
            s["post2_complete"] = s["post2_complete"]
        wer_numbers([s for s in recorded], run, audio_dir, P)
        P("")

    P("UNIT SPEED  engine seconds per audio second for every unit (1.0 = real time), by revision")
    for t, name, idx, rev, took, dev, rs, pk, lim, dth in unit_done[:400]:
        s = names.get(name)
        if not s:
            continue
        u = sql(s["dir"] / "transcript.db", "SELECT start_ms, end_ms FROM units WHERE idx=? AND job IN (SELECT id FROM jobs WHERE revision=?)", (idx, rev))
        if not u:
            continue
        dur = (u[0][1] - u[0][0]) / 1000.0
        P(f"  {title_of.get(name, name)} unit {idx} of {rev}: {dur:.0f} s of audio in {took:.0f} s on {dev} -> {took / dur:.2f}, {rs} restart(s), peak {pk} of {lim} MiB, done at {hms(t)}")
    P("")

    sf = run / "process-samples.csv"
    P("MACHINE LOAD  the per-process sampler (rows every 10 s)")
    if sf.exists():
        rows = []
        with open(sf, encoding="ascii") as fh:
            for r in csv.DictReader(fh):
                try:
                    rows.append({k: float(v) for k, v in r.items() if k != "gpu_by_adapter"})
                except ValueError:
                    pass
        if rows:
            cols = ["host_cpu", "engine_cpu", "app_cpu", "webview_cpu", "player_cpu", "script_cpu", "other_cpu"]
            P(f"  {len(rows)} samples over {(rows[-1]['unix_s'] - rows[0]['unix_s']) / 3600.0:.1f} h; mean CPU % of the host: " + ", ".join(f"{c.replace('_cpu', '')} {sum(r[c] for r in rows) / len(rows):.1f}" for c in cols))
            P(f"  other (outside our processes) above 15% in {sum(1 for r in rows if r['other_cpu'] > 15)} of {len(rows)} samples; highest {max(r['other_cpu'] for r in rows):.0f}%")
    else:
        P("  no process-samples.csv")
    P("")

    # ---------------------------------------------------------------- timeline
    off = notes.get("clock_offset_s") or 0
    lanes = []
    for s in sorted(sess, key=lambda s: day_start.get(s["title"], 1e12)):
        bars = []
        if s["title"] in day_start and s["title"] in day_stop and not s["imported"]:
            bars.append(("record", day_start[s["title"]], day_stop[s["title"]]))
        for t, name, idx, rev, took, dev, rs, pk, lim, dth in unit_done:
            if name == s["name"]:
                bars.append(("post-1" if rev == "post-1" else "post-2", t - took, t))
        ticks = []
        for name, t, rb, fin, outstanding, basis in est_rows:
            if name == s["name"] and rb:
                ticks.append((rb, "ready by at Stop"))
        lanes.append({"label": s["title"] + (" (copy)" if s["imported"] else ""), "bars": bars, "ticks": ticks})
    if qlog:
        t0 = 10 * 3600 - 600
        t1 = max([t for t, _ in qlog] + [0]) + 600
        marks = []
        if planned:
            marks.append((planned[0], "night planned"))
        timeline_svg(run / "timeline.svg", lanes, marks, t0, t1)
        P(f"timeline: {run / 'timeline.svg'} (copied next to this file)")
        P("")

    # ---------------------------------------------------------------- verdicts
    P("CHECKS")
    failed = 0
    info = 0
    for name, ok, detail in checks:
        mark = "PASS" if ok else ("INFO" if ok is None else "FAIL")
        failed += 1 if ok is False else 0
        # The estimate lines of the rehearsal are information by design (two-minute sessions): they do not make it incomplete.
        info += 1 if (ok is None and not (mode == "rehearse" and name.startswith("rolling transcripts outstanding"))) else 0
        P(f"  [{mark}] {name}" + (f": {detail}" if detail else ""))
    P("")
    if notes.get("drivers_changed"):
        P("VERDICT: INVALID (the display driver changed during the run; no verdict)")
    elif partial:
        P(f"VERDICT: INVALID as a day simulation (partial run 1, kept as information; the checks above are of what ran: {len(checks) - failed - info} passed, {failed} failed, {info} information)")
    else:
        P(f"VERDICT: {'PASS' if failed == 0 and not info else ('FAIL' if failed else 'INCOMPLETE (a session was INVALID or is information only: see the lines above)')} ({len(checks) - failed - info} passed, {failed} failed, {info} information or invalid)")
    P("Speed, 'ready by' and 10% differences on the emulation profile are low-tier findings with numbers, not code failures (owner, 17 and 18 Nov 2026); the real laptops repeat the run.")
    P("")
    P("Scheduler log (every line, IST time first):")
    for t, x in qlog:
        P(f"  {hms(t)} {x}")
    out.write_text("\n".join(L) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
