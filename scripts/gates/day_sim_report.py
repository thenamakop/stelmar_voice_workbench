"""Writes the evidence text of a day-simulation smoke run from the run folder (nothing is edited by hand).

    eval/.venv/Scripts/python scripts/gates/day_sim_report.py <run folder> <out.txt>

Reads: report.json (the app's report), queue.log (the scheduler log with IST times), script-notes.json (what the script
did: the engine kill, the cable, the clock offset), day-plan.json, and for every session folder under rec/: parts.json,
the segment files post-1.jsonl and post-2.jsonl, language-log.jsonl, transcript.db (jobs and units) and
work/*/batch-request-*.json (the requests the engines were given). It states facts and a verdict line for each path the
smoke has to exercise; whether a path "worked", was "fixed" or stays "open" is decided in the GATES row, by a person.
"""

from __future__ import annotations

import difflib
import json
import re
import sqlite3
import sys
from datetime import datetime
from pathlib import Path


def jload(p: Path):
    try:
        return json.loads(p.read_text(encoding="utf-8-sig"))
    except Exception:
        return None


def jsonl(p: Path) -> list[dict]:
    out = []
    if p.exists():
        for line in p.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                try:
                    out.append(json.loads(line))
                except Exception:
                    pass
    return out


def mib(b) -> str:
    return "unknown" if b is None else f"{b / (1 << 20):.0f} MiB"


def find_sessions(rec: Path) -> list[Path]:
    # import-source/ holds the copies the script made for import_session; the imported session is the one beside it.
    return sorted({p.parent for p in rec.rglob("parts.json") if "import-source" not in p.parts})


def sql(db: Path, q: str, args=()) -> list[tuple]:
    if not db.exists():
        return []
    try:
        con = sqlite3.connect(f"file:{db.as_posix()}?mode=ro", uri=True)
        try:
            return con.execute(q, args).fetchall()
        finally:
            con.close()
    except Exception as e:  # noqa: BLE001
        return [("sqlite error", str(e))]


def seg_checks(segs: list[dict]) -> dict:
    segs = sorted(segs, key=lambda s: s["start_ms"])
    overlaps = 0
    dup = 0
    for a, b in zip(segs, segs[1:]):
        if b["start_ms"] == a["start_ms"]:
            dup += 1
        if b["start_ms"] < a["end_ms"] - 1:
            overlaps += 1
    return {"count": len(segs), "overlaps": overlaps, "duplicate_starts": dup}


def main() -> int:
    run = Path(sys.argv[1])
    out = Path(sys.argv[2])
    L: list[str] = []
    P = L.append

    report = jload(run / "report.json") or {}
    notes = jload(run / "script-notes.json") or {}
    plan = jload(run / "day-plan.json") or {}
    qlog = (run / "queue.log").read_text(encoding="utf-8", errors="replace").splitlines() if (run / "queue.log").exists() else []
    app_log = report.get("queue", {}).get("log", [])

    P("DAY-SIMULATION SMOKE (not a gate)")
    P(f"run folder: {run.name}")
    P(f"profile in force: {notes.get('profile')}")
    P(f"app exit code: {notes.get('emu_exit')}, scheduler clock offset {notes.get('clock_offset_s')} s (the first session maps to 10:00 IST), parts of {notes.get('part_seconds')} s, night pass time box {notes.get('night_minutes')} min")
    q = report.get("queue", {})
    P(f"profile reads by the scheduler: {q.get('profile_reads')}, mean {q.get('profile_read_mean_us')} us, slowest {q.get('profile_read_max_us')} us (the profile in use is read at every scheduling decision, no cache)")
    P(f"configuration the queue worked in: {q.get('configuration')}; forced hybrid: {report.get('forced_hybrid')}")
    P("")
    P("Script notes (progress):")
    for l in notes.get("progress", []):
        P(f"  {l}")
    P("")

    # ------------------------------------------------------------------ sessions
    rec = run / "rec"
    sessions = find_sessions(rec)
    info: dict[str, dict] = {}
    P("Sessions found under rec/:")
    for d in sessions:
        parts = jload(d / "parts.json") or {}
        plist = parts.get("parts", [])
        title = (jload(d / "session.json") or {}).get("title", "?")
        rates = sorted({p.get("sample_rate") for p in plist})
        total = sum(p.get("duration_ms", 0) for p in plist)
        info[d.name] = {"dir": d, "title": title, "parts": plist, "audio_ms": total}
        P(f"  {d.name}  title {title!r}  parts {len(plist)}  sample rates {rates}  channels {sorted({p.get('channels') for p in plist})}  audio {total / 1000:.1f} s")
    P("")

    # ------------------------------------------------------------------ per session facts
    for name, s in info.items():
        d: Path = s["dir"]
        P(f"== {name} ({s['title']})")
        db = d / "transcript.db"
        jobs = sql(db, "SELECT id, kind, revision, state, cursor_ms, models_by_lang, languages, started_ms, finished_ms, error FROM jobs ORDER BY id")
        for j in jobs:
            P(f"  job {j[0]} {j[1]} {j[2]} state {j[3]} cursor {j[4]} ms, started {j[7]}, finished {j[8]}, error {j[9]}")
            P(f"    languages {j[6]}, models {j[5]}")
            units = sql(db, "SELECT idx, start_ms, end_ms, state, device, cpu_attempts, gpu_attempts, seam, last_error FROM units WHERE job=? ORDER BY idx", (j[0],))
            for u in units:
                P(f"    unit {u[0]}: {u[1] / 1000:.1f} s to {u[2] / 1000:.1f} s ({(u[2] - u[1]) / 1000:.1f} s), state {u[3]}, device {u[4]}, attempts cpu {u[5]} gpu {u[6]}, seam {u[7]}, last error {u[8]}")
        plan_json = jload(d / "work" / "plan.json")
        if plan_json:
            for pe in plan_json.get("parts", []):
                pauses = pe.get("pauses")
                P(f"  part {pe.get('part')}: {'no pauses recorded' if pauses is None else str(len(pauses)) + ' pauses'} (from wb-engine chunks)")
        for rev in ("post-1", "post-2"):
            segs = jsonl(d / f"{rev}.jsonl")
            if not segs:
                continue
            c = seg_checks(segs)
            end = max(x["end_ms"] for x in segs)
            covered = sum(x["end_ms"] - x["start_ms"] for x in segs)
            langs: dict[str, int] = {}
            mixed = 0
            for x in segs:
                langs[x["lang"]] = langs.get(x["lang"], 0) + 1
                mixed += 1 if x.get("mixed") else 0
            devs = sorted({str(x.get("device")) for x in segs})
            models = sorted({x.get("model", "") for x in segs})
            P(f"  {rev}.jsonl: {c['count']} segments, overlaps {c['overlaps']}, duplicate starts {c['duplicate_starts']}, last end {end / 1000:.1f} s of {s['audio_ms'] / 1000:.1f} s, speech {covered / 1000:.1f} s, languages {langs}, mixed {mixed}, devices {devs}, models {models}")
            first_created = min((x.get("created_at", "") for x in segs), default="")
            P(f"    first segment committed at {first_created}")
        P("")

    # ------------------------------------------------------------------ path 1: chunks on a real 48 kHz part
    P("PATH 1  wb-engine chunks on a real recorder part (48 kHz)")
    chunk_lines = [l for l in qlog + app_log if "chunks" in l.lower()]
    for l in chunk_lines[:40]:
        P(f"  {l}")
    rates = {r for s in info.values() for r in (p.get("sample_rate") for p in s["parts"])}
    plans_ok = []
    for name, s in info.items():
        pj = jload(s["dir"] / "work" / "plan.json") or {}
        for pe in pj.get("parts", []):
            plans_ok.append((name, pe.get("part"), pe.get("pauses")))
    with_pauses = [x for x in plans_ok if x[2]]
    P(f"  part sample rates: {sorted(rates)}; parts with pauses found: {len(with_pauses)} of {len(plans_ok)}")
    bad = [l for l in chunk_lines if "could not find the pauses" in l]
    P(f"  VERDICT: {'PASS' if with_pauses and 48000 in rates and not bad else 'CHECK'} (pauses found for {len(with_pauses)} parts; failures: {len(bad)})")
    P("")

    # ------------------------------------------------------------------ path 2: batch engine through the queue
    P("PATH 2  the real batch engine through the queue (rolling pass, Stop, the rest cut)")
    sess_report = {x["title"]: x for x in report.get("sessions", [])}
    for name, s in info.items():
        r = sess_report.get(s["title"])
        segs = jsonl(s["dir"] / "post-1.jsonl")
        stopped = (r or {}).get("stopped_ist")
        before = []
        if stopped:
            try:
                ts = datetime.fromisoformat(stopped)
                for x in segs:
                    ca = x.get("created_at", "")
                    if ca:
                        try:
                            if datetime.fromisoformat(ca) < ts:
                                before.append(x)
                        except Exception:
                            pass
            except Exception:
                pass
        P(f"  {name}: Stop at {stopped}; segments committed before Stop: {len(before)} of {len(segs)}")
    planned = [l for l in qlog if "unit(s) planned after part" in l]
    P(f"  units planned while the session was still recording: {len(planned)}")
    for l in planned:
        P(f"    {l}")
    done_lines = [l for l in qlog if re.search(r"unit \d+ of post-\d done", l)]
    P(f"  units done: {len(done_lines)}")
    for l in done_lines:
        P(f"    {l}")
    P("")

    # peak memory
    P("ENGINE MEMORY (peak against the limit the gate granted; the smoke fails above 70%)")
    worst = 0.0
    for l in qlog + app_log:
        m = re.search(r"engine peak (\d+) MiB against limit (\d+) MiB", l)
        if m:
            ratio = int(m.group(1)) / max(1, int(m.group(2)))
            worst = max(worst, ratio)
    P(f"  worst peak/limit over all units: {worst * 100:.1f}%  -> {'PASS' if worst <= 0.70 else 'FAIL'}")
    deaths = [l for l in qlog if "memory-limit death" in l or "MemoryLimit" in l]
    P(f"  memory-limit deaths logged: {len(deaths)}")
    P("")

    # ------------------------------------------------------------------ path 3: import
    P("PATH 3  import_session of a copy of a finished session")
    actions = report.get("actions", [])
    for a in actions:
        P(f"  action {a.get('do')} on {a.get('session')} at {a.get('at_ist')}: {'ok' if a.get('ok') else 'ERROR ' + str(a.get('error'))} {a.get('detail') or ''}")
    orig = next((s for n, s in info.items() if not n.endswith("-copy") and s["title"] == (plan.get("actions") or [{}])[-1].get("from")), None)
    imp = next((s for n, s in info.items() if n.endswith("-copy")), None)
    if orig and imp:
        a = [x["text"] for x in sorted(jsonl(orig["dir"] / "post-1.jsonl"), key=lambda x: x["start_ms"])]
        b = [x["text"] for x in sorted(jsonl(imp["dir"] / "post-1.jsonl"), key=lambda x: x["start_ms"])]
        ratio = difflib.SequenceMatcher(None, " ".join(a), " ".join(b)).ratio() if a and b else 0.0
        P(f"  original {len(a)} segments, imported copy {len(b)} segments, text similarity {ratio:.3f}")
        P(f"  VERDICT: {'PASS' if b and ratio > 0.9 else 'CHECK'}")
    else:
        P("  VERDICT: CHECK (no imported copy found)")
    P("")

    # ------------------------------------------------------------------ path 4: language lock
    P("PATH 4  set_language_lock: a lock inside a unit reaches that unit's lang_log")
    for name, s in info.items():
        ll = jsonl(s["dir"] / "language-log.jsonl")
        if not ll:
            continue
        P(f"  {name}: language-log.jsonl = {ll}")
        reqs = sorted((s["dir"] / "work").glob("*/batch-request-*.json"))
        for rq in reqs:
            j = jload(rq) or {}
            lg = j.get("lang_log")
            P(f"    {rq.parent.name}/{rq.name}: unit {j.get('unit_start_ms', j.get('start_ms'))}..{j.get('unit_end_ms', j.get('end_ms'))} lang_log {lg}")
    P("")

    # ------------------------------------------------------------------ path 5: pause and resume; path 6: kill; path 7: night
    # ------------------------------------------------------------------ who used the machine during each unit
    P("UNIT WINDOWS  CPU (percent of the whole host) and GPU (percent of one engine) by process, from process-samples.csv (a row every 10 s; the sampler is part of the script's own cost)")
    sf = run / "process-samples.csv"
    off = notes.get("clock_offset_s")
    if not sf.exists() or off is None:
        P("  no process-samples.csv (the run predates the sampler)")
    else:
        import csv
        samples = []
        with open(sf, encoding="ascii") as fh:
            for r in csv.DictReader(fh):
                try:
                    samples.append({k: (float(v) if k not in ("gpu_by_adapter",) else v) for k, v in r.items()})
                except ValueError:
                    pass
        cols = ["host_cpu", "engine_cpu", "app_cpu", "webview_cpu", "player_cpu", "script_cpu", "other_cpu", "engine_gpu", "app_gpu", "webview_gpu", "other_gpu"]
        P(f"  {len(samples)} samples")
        P("  unit                         wall s   rows | CPU: host engine app webview player sampler other | GPU: engine app webview other | the engine's adapter: engine app webview other")
        for l in app_log:
            m = re.match(r"^(\d+) (.+?): unit (\d+) of (post-\d+) done in ([\d.]+) s on (\w+)", l)
            if not m:
                continue
            end = int(m.group(1)) / 1000 - off
            took = float(m.group(5))
            win = [x for x in samples if end - took <= x["unix_s"] <= end]
            if not win:
                P(f"  {m.group(2)} unit {m.group(3)} of {m.group(4)}: no samples in its window")
                continue
            mean = {c: sum(x[c] for x in win) / len(win) for c in cols}
            # The adapter the engine ran on: the one where the engine's GPU share is highest.
            best, best_v = None, -1.0
            acc = {}
            for x in win:
                for part in str(x["gpu_by_adapter"]).split(";"):
                    if "=" not in part:
                        continue
                    k, v = part.split("=", 1)
                    vals = [float(t) for t in v.split("/")]
                    a = acc.setdefault(k, [0.0] * 4)
                    for i in range(4):
                        a[i] += vals[i] / len(win)
            for k, a in acc.items():
                if a[0] > best_v:
                    best, best_v = k, a[0]
            ad = f"{best}: " + " ".join(f"{v:.0f}" for v in acc[best]) if best else "-"
            P(f"  {m.group(2)} unit {m.group(3)} of {m.group(4)}  {took:7.1f}  {len(win):5d} | " + " ".join(f"{mean[c]:5.1f}" for c in cols[:7]) + " | " + " ".join(f"{mean[c]:5.1f}" for c in cols[7:]) + f" | {ad}")
    P("")

    P("ESTIMATES  the first 'ready by' after Stop and the night plan, with the basis each rests on (the provisional emu-low detection allowance until a unit of a session with several languages has finished)")
    q = report.get("queue", {})
    for l in qlog + app_log:
        if re.search(r": stopped\. |night pass planned|night pass skipped", l):
            P(f"  {l}")
    P(f"  at the end: basis '{q.get('estimate_basis')}'; several languages: factor {q.get('speed_factor')} from {q.get('speed_units')} unit(s); one language: factor {q.get('speed_factor_one_language')} from {q.get('speed_units_one_language')} unit(s); allowances rolling {q.get('detection_allowance_rolling')}, night {q.get('detection_allowance_night')}")
    P("")
    P("PATH 5  queue_pause and queue_resume")
    for l in qlog:
        if re.search(r"operator (paused|resumed)|waiting, not processing|processing may continue|paused:|resumed \(", l):
            P(f"  {l}")
    P("")
    P("PATH 6  one engine killed from outside, then resumed")
    P(f"  script: {notes.get('kill')}")
    ev = run / "script-events.log"
    if ev.exists():
        for l in ev.read_text(encoding="utf-8", errors="replace").splitlines():
            P(f"  {l}")
    for l in qlog:
        if re.search(r"SMOKE:|failed on the|trying again|restart|Restart|Crash|crash|Exited|exited", l):
            P(f"  {l}")
    P("")
    P("PATH 7  a short night pass")
    for l in qlog:
        if "night" in l.lower():
            P(f"  {l}")
    for name, s in info.items():
        n2 = len(jsonl(s["dir"] / "post-2.jsonl"))
        if n2:
            P(f"  {name}: post-2.jsonl {n2} segments")
        db = s["dir"] / "transcript.db"
        for r in sql(db, "SELECT name, complete, is_default FROM revisions ORDER BY name"):
            P(f"  {name}: revision {r[0]}: complete {bool(r[1])}, default {bool(r[2])}")
        for j in sql(db, "SELECT id, kind, revision, state, cursor_ms FROM jobs WHERE kind='night'"):
            P(f"  {name}: night job {j[0]} (revision {j[2]}): state {j[3]}, committed to {j[4] / 1000:.1f} s")
            for u in sql(db, "SELECT idx, start_ms, end_ms, state FROM units WHERE job=? ORDER BY idx", (j[0],)):
                P(f"    night unit {u[0]}: {u[1] / 1000:.1f} s to {u[2] / 1000:.1f} s, state {u[3]}")
    P("")

    P("Scheduler log (every line, IST time first):")
    for l in qlog:
        P(f"  {l}")
    out.write_text("\n".join(L) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
