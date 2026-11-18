"""Helper of scripts/gates/speed-diagnosis.ps1 (owner, 17 Nov 2026, steps 2 and 3 of the speed and memory diagnosis).

    python -I speed_diag.py prep   SESSION_DIR WORK_DIR BENCH_CLIP
    python -I speed_diag.py report WORK_DIR OUT_FILE [--header FILE]

prep   writes the request files the script runs: the queue's own rolling request (Q) and night request (N1) from the
       smoke run's unit folders, the same rolling unit as the profile cell asks it (R: one language, no detection), the
       night unit with one model and one language for 120 s (N2), and the bench request of the profile cell (P).
report reads each run's per-second trace (CSV), its engine events and its idle check, and writes the evidence file.
"""

from __future__ import annotations

import argparse
import csv
import glob
import json
import statistics
import sys
from pathlib import Path


def load(p: Path):
    return json.loads(p.read_text(encoding="utf-8-sig"))


def dump(p: Path, o) -> None:
    p.write_text(json.dumps(o, ensure_ascii=False, indent=1), encoding="utf8")


def prep(a) -> int:
    session = Path(a.session)
    work = Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    wk = session / "work"
    q = load(wk / "post-1-unit-0001" / "batch-request-0.json")
    n1 = load(wk / "post-2-unit-0000" / "batch-request-0.json")
    gpu = q["device"]
    dump(work / "req-Q.json", q)
    r = json.loads(json.dumps(q))
    r["languages"] = ["hi"]
    r["detect_language"] = False
    dump(work / "req-R.json", r)
    dump(work / "req-N1.json", n1)
    n2 = json.loads(json.dumps(n1))
    night = n1["models_by_lang"]["hi"]
    n2["models_by_lang"] = {"*": night, "hi": night}
    n2["languages"] = ["hi"]
    n2["detect_language"] = False
    n2["units"] = [{"index": 0, "start_ms": 0, "end_ms": 120000}]
    dump(work / "req-N2.json", n2)
    turbo = q["models_by_lang"]["hi"]
    p = {
        "type": "bench",
        "model": turbo,
        "vad_model": q["vad_model"],
        "lang": "hi",
        "clip": a.clip,
        "mode": "rolling",
        "device": gpu,
        "threads": q["threads"],
        "runs": 1,
    }
    dump(work / "req-P.json", p)
    print(json.dumps({"gpu": gpu, "threads": q["threads"], "files": sorted(x.name for x in work.glob("req-*.json"))}))
    return 0


def events(path: Path):
    out = []
    if path.exists():
        for line in path.read_text(encoding="utf8", errors="replace").splitlines():
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
    return out


def slope_per_min(ts, ys) -> float:
    """Least-squares slope of y over t, in y units per minute."""
    if len(ts) < 3:
        return 0.0
    mt, my = statistics.fmean(ts), statistics.fmean(ys)
    den = sum((t - mt) ** 2 for t in ts)
    if den == 0:
        return 0.0
    return sum((t - mt) * (y - my) for t, y in zip(ts, ys)) / den * 60


def at(ts, ys, frac):
    if not ts:
        return 0.0
    target = ts[-1] * frac
    best = min(range(len(ts)), key=lambda i: abs(ts[i] - target))
    return ys[best]


LABELS = {
    "P1": "P1  profile cell: bench, Hindi 60 s clip, turbo q5_0, Vulkan UHD, 2 threads, rolling (first)",
    "Q": "Q   queue's rolling unit: batch, en+hi with detection on every chunk, 288 s, the request post-1 unit 1 ran",
    "R": "R   the same unit with the cell's conditions: batch, Hindi only, no language detection",
    "P2": "P2  profile cell again (drift check)",
    "N2": "N2  night model only (large-v3 q5_0), Hindi only, no detection, first 120 s",
    "N1": "N1  queue's night unit: batch, en (turbo) + hi (large-v3 q5_0) with detection, 295 s, the request post-2 unit 0 ran",
}


def report(a) -> int:
    work = Path(a.work)
    lines: list[str] = []
    if a.header and Path(a.header).exists():
        lines += Path(a.header).read_text(encoding="utf8").splitlines()
        lines.append("")
    lines.append("Idle checks before each run (total CPU of the host averaged over 60 s must be under 10%, and no other pwsh, cargo, robocopy, python or wb-* process busy)")
    for k in LABELS:
        f = work / f"idle-{k}.json"
        if not f.exists():
            continue
        j = load(f)
        top = ", ".join(f"{t['name']} {t['cpu_s']:.1f} CPU-s" for t in j["top"][:4]) or "nothing above 0.5 CPU-s"
        lines.append(
            f"  {k}: {j['at']} host CPU {j['cpu_avg']:.1f}% (max {j['cpu_max']:.0f}%) over {j['seconds']} s, attempts {j['attempts']}, "
            f"busy {', '.join(j['busy']) or 'none'}; top: {top}; free RAM {j['free_ram_gb']} GB -> {'IDLE' if j['ok'] else 'NOT IDLE'}"
        )
    lines.append("")
    rows = {}
    for k in LABELS:
        tr = work / f"{k}.trace.csv"
        if not tr.exists():
            continue
        ev = events(work / f"{k}.out")
        ts, ws, cm, ecpu, hcpu = [], [], [], [], []
        with open(tr, encoding="utf8") as fh:
            for row in csv.DictReader(fh):
                if row["commit_mib"] == "":
                    continue
                ts.append(float(row["t_s"]))
                ws.append(float(row["ws_mib"]))
                cm.append(float(row["commit_mib"]))
                ecpu.append(float(row["engine_cpu_pct"] or 0))
                hcpu.append(float(row["host_cpu_pct"] or 0))
        meta = load(work / f"{k}.run.json")
        ready = next((e for e in ev if e.get("event") == "ready"), None)
        stats = [e for e in ev if e.get("event") == "stats"]
        bench = next((e for e in ev if e.get("event") == "bench"), None)
        done = next((e for e in ev if e.get("event") == "done"), None)
        rows[k] = dict(meta=meta, ts=ts, ws=ws, cm=cm, ecpu=ecpu, hcpu=hcpu, ready=ready, stats=stats, bench=bench, done=done, ev=ev)
    lines.append("Runs (every one under wb-emulate --profile emu-low; the engine is the app's Vulkan build; engine process at below-normal priority like the queue's)")
    for k, r in rows.items():
        m = r["meta"]
        lines.append(f"  {LABELS[k]}")
        lines.append(f"      exit {m['exit']}, wall {m['wall_s']:.1f} s, started {m['started']}, engine sha256 {m['engine_sha256'][:16]}")
        if r["bench"]:
            b = r["bench"]
            lines.append(
                f"      bench: RTF {b['rtf_median']:.3f} (warm-up {b['warmup_rtf']:.3f}), clip {b['clip_seconds']:.1f} s, load {b['load_s']:.1f} s, "
                f"{b['segments']} segments, peak commit {b['peak_commit_bytes'] / 2**20:.0f} MiB, working set {b['peak_working_set_bytes'] / 2**20:.0f} MiB, device {b['device_name']}"
            )
        if r["ready"]:
            lines.append(f"      ready: models {r['ready']['models']}, load {r['ready']['load_s']:.1f} s, device {r['ready']['device_name']}, threads {r['ready']['threads']}")
        for s in r["stats"]:
            audio = s["audio_ms"] / 1000
            lines.append(
                f"      stats: unit {s['unit']}, {s['chunks']} chunks, audio {audio:.1f} s, unit time {s['decode_s']:.1f} s, "
                f"RTF {s['decode_s'] / max(audio, 0.001):.3f} (engine reports {s['rtf']:.3f}), detections {s['detections']}, both ways {s['both_ways']}, peak working set {s['peak_working_set_bytes'] / 2**20:.0f} MiB"
            )
        if r["done"]:
            d = r["done"]
            summ = d.get("summary", d)
            lines.append(
                f"      done: segments {summ.get('segments')}, chunks {summ.get('chunks')}, retried {summ.get('retried_chunks')}, unclear {summ.get('unclear_chunks')}, "
                f"low coverage {summ.get('low_coverage_chunks')}, truncated {summ.get('truncated_chunks')}, elapsed {summ.get('elapsed_s')}"
            )
        warns = [e for e in r["ev"] if e.get("event") == "warning"]
        if warns:
            codes: dict[str, int] = {}
            for w in warns:
                codes[w["code"]] = codes.get(w["code"], 0) + 1
            lines.append("      warnings: " + ", ".join(f"{c} x{n}" for c, n in sorted(codes.items())))
        ts, ws, cm = r["ts"], r["ws"], r["cm"]
        if ts:
            pk = max(cm)
            t90 = next((t for t, c in zip(ts, cm) if c >= 0.9 * pk), ts[-1])
            tail = [(t, c) for t, c in zip(ts, cm) if t >= ts[-1] * 0.3]
            sl = slope_per_min([t for t, _ in tail], [c for _, c in tail])
            sw = slope_per_min([t for t, _ in tail], [w for t, w in zip(ts, ws) if t >= ts[-1] * 0.3])
            lines.append(
                f"      trace ({len(ts)} samples, 1 per second): peak commit {pk:.0f} MiB, peak working set {max(ws):.0f} MiB, 90% of the peak commit reached at {t90:.0f} s ({t90 / ts[-1] * 100:.0f}% of the run); "
                f"commit at 25/50/75/100% of the run {at(ts, cm, .25):.0f}/{at(ts, cm, .5):.0f}/{at(ts, cm, .75):.0f}/{cm[-1]:.0f} MiB; "
                f"slope over the last 70% {sl:+.1f} MiB/min (commit), {sw:+.1f} MiB/min (working set)"
            )
            lines.append(
                f"      CPU: engine {statistics.fmean(r['ecpu']):.0f}% of one core on average (max {max(r['ecpu']):.0f}%), host total {statistics.fmean(r['hcpu']):.1f}% on average (max {max(r['hcpu']):.0f}%) during the run"
            )
    lines.append("")
    lines.append("Arithmetic (no reading; the reading goes in a note that names its author)")
    def rtf_of(k):
        r = rows.get(k)
        if not r:
            return None
        if r["bench"]:
            return r["bench"]["rtf_median"]
        if r["stats"]:
            s = r["stats"][0]
            return s["decode_s"] / max(s["audio_ms"] / 1000, 0.001)
        return None
    for a_, b_, what in (("Q", "R", "queue's conditions over the cell's conditions, same audio"), ("R", "P1", "batch with the cell's conditions over the cell's bench"), ("P1", "P2", "drift of the bench between the first and the last run"), ("N1", "N2", "queue's night unit over night model alone (different lengths)")):
        x, y = rtf_of(a_), rtf_of(b_)
        if x and y:
            lines.append(f"  {a_} / {b_}: {x:.3f} / {y:.3f} = {x / y:.2f}   ({what})")
    prof = {"P1": 0.696, "P2": 0.696}
    for k in ("P1", "P2"):
        x = rtf_of(k)
        if x:
            lines.append(f"  {k} against the profile's 0.696 for this cell: {x / 0.696:.2f}")
    if a.compare:
        lines.append("")
        lines.append("Against the earlier runs (a run more than 5% faster than its earlier figure means the earlier figure was slowed by something that is now gone)")
        for item in a.compare.split(","):
            k, _, v = item.partition("=")
            k = k.strip()
            x = rtf_of(k)
            if x is None or not v:
                continue
            earlier = float(v)
            diff = (x - earlier) / earlier * 100
            lines.append(f"  {k}: {x:.3f} now against {earlier:.3f} earlier = {diff:+.1f}% ({'MORE THAN 5% FASTER' if diff < -5 else 'within 5% or slower'})")
    text = "\n".join(lines) + "\n"
    Path(a.out).write_text(text, encoding="utf8")
    sys.stdout.reconfigure(encoding="utf8")
    print(text)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("prep")
    p.add_argument("session")
    p.add_argument("work")
    p.add_argument("clip")
    p = sub.add_parser("report")
    p.add_argument("work")
    p.add_argument("out")
    p.add_argument("--header", default="")
    p.add_argument("--compare", default="", help="earlier RTFs, e.g. P1=0.708,P2=0.709,Q=1.011")
    a = ap.parse_args()
    return prep(a) if a.cmd == "prep" else report(a)


if __name__ == "__main__":
    sys.exit(main())
