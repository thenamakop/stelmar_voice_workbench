"""Step 1 of the speed diagnosis (owner, 17 Nov 2026): for every rolling and night unit of a day-simulation run, what the
queue asked the engine to do and how long it took, next to the profile cell behind the speed the queue was promised.

    eval/.venv/Scripts/python -I scripts/gates/speed_table.py RUN_DIR [--out FILE]

RUN_DIR is a day-simulation run folder (rec/<date>/<session>/work/<rev>-unit-NNNN/batch-request-0.json, engine.log and
the unit's staging file, report.json and data/capability.json). Nothing is measured here: the table reads what the run
left behind. Wall time is the supervisor's, from the queue log ("unit N of post-R done in S s").
"""

from __future__ import annotations

import argparse
import glob
import json
import re
import sys
from pathlib import Path


def mib(b: float) -> int:
    return round(b / 2**20)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--out", default="")
    a = ap.parse_args()
    run = Path(a.run)
    cap = json.loads((run / "data" / "capability.json").read_text(encoding="utf8"))
    report = json.loads((run / "report.json").read_text(encoding="utf8"))
    log = report["queue"]["log"]
    done = {}
    for l in log:
        m = re.search(r"unit (\d+) of (post-\d+) done in ([\d.]+) s on (\w+) \((\d+) restart\(s\), engine peak (\d+) MiB against limit (\d+) MiB", l)
        if m:
            done[(m.group(2), int(m.group(1)))] = dict(
                wall=float(m.group(3)), dev=m.group(4), restarts=int(m.group(5)), peak=int(m.group(6)), limit=int(m.group(7))
            )
    meas = {(m["model"], m["lang"], m["mode"], json.dumps(m["config"], sort_keys=True)): m for m in cap["measurements"]}
    out: list[str] = []
    out.append(f"Speed table of {run.name}: what the queue ran, against the profile cell behind its promised speed")
    out.append(f"profile: {cap['emulated']}, created {cap['created_at']}, check {cap['check']['kind']}")
    out.append("")
    out.append("Per unit (the queue's side)")
    hdr = "  unit            audio s  speech s  chunks  detect  retries  device                   thr  model(s)                                  beam/best  wall s   RTF   peak/limit MiB"
    out.append(hdr)
    units = sorted(glob.glob(str(run / "rec" / "*" / "*" / "work" / "post-*-unit-*" / "batch-request-0.json")))
    cells_used = set()
    for req_path in units:
        d = Path(req_path).parent
        m = re.match(r"(post-\d+)-unit-(\d+)", d.name)
        rev, idx = m.group(1), int(m.group(2))
        req = json.loads(Path(req_path).read_text(encoding="utf8"))
        u = req["units"][0]
        audio_s = (u["end_ms"] - u["start_ms"]) / 1000
        st = req["settings"]
        dev = req["device"]
        devtxt = f"gpu {dev['index']}" if dev["kind"] == "gpu" else "cpu"
        models = sorted({v["id"] for v in req["models_by_lang"].values()})
        stage = next(iter(glob.glob(str(d.parent / rev / f"unit-{idx:04d}.jsonl"))), None)
        segs, speech = 0, 0.0
        if stage:
            for line in open(stage, encoding="utf8"):
                j = json.loads(line)
                if "text" in j:
                    segs += 1
                    speech += (j["end_ms"] - j["start_ms"]) / 1000
        elog = (d / "engine.log").read_text(encoding="utf8", errors="replace")
        retries = len(re.findall(r"chunk_retry", elog))
        dn = done.get((rev, idx))
        wall = f"{dn['wall']:.0f}" if dn else "-"
        rtf = f"{dn['wall'] / audio_s:.2f}" if dn else "-"
        pk = f"{dn['peak']}/{dn['limit']}" if dn else "-"
        out.append(
            f"  {rev} u{idx}  {audio_s:8.0f}  {speech:8.0f}  {segs:6d}  {('yes' if req.get('detect_language') else 'no'):6s}  {retries:7d}  "
            f"{devtxt:<24s} {req['threads']:3d}  {','.join(models):<42s} {st['beam_size']}/{st['best_of']}       {wall:>6s}  {rtf:>5s}  {pk}"
        )
        for lang in req["languages"]:
            cells_used.add((req["models_by_lang"].get(lang, req["models_by_lang"]["*"])["id"], lang, "rolling", json.dumps({"kind": dev["kind"], **({"index": dev["index"]} if dev["kind"] == "gpu" else {})}, sort_keys=True)))
    out.append("")
    out.append("The same settings in every unit: beam 5, best-of 5, temperature fallback on, previous-text context on (200 characters),")
    out.append("max chunk 15 s, sentence split on, VAD threshold 0.5, flash attention on, language detection on every chunk of 0.3 s or more")
    out.append("when the session has two or more languages and no operator lock. Languages of the units above: " + ", ".join(sorted({l for p in units for l in json.loads(Path(p).read_text(encoding='utf8'))['languages']})))
    out.append("")
    out.append("The profile cells (what the speed promise comes from)")
    out.append("  cell: model, language, device, threads: RTF (warm-up RTF), clip, load s, peak commit MiB, method")
    for k in sorted(cells_used):
        m = meas.get(k)
        if not m:
            out.append(f"  {k[0]} {k[1]} {k[3]}: no such cell in the profile")
            continue
        out.append(
            f"  {m['model']} {m['lang']} {m['engine_build']} {m['device_name']} {m['threads']} thr: RTF {m['rtf']:.3f} (warm-up {m['warmup_rtf']:.3f}), {m['clip_seconds']:.0f} s clip, load {m['load_s']:.1f} s, "
            f"commit {mib(m['peak_commit_bytes'])} MiB, working set {mib(m['peak_working_set_bytes'])} MiB; {m['method']}"
        )
    shown = {k[0] for k in cells_used}
    night = [m for m in cap["measurements"] if m["model"] == "large-v3-q5_0" and m["model"] not in shown]
    for m in night:
        out.append(
            f"  {m['model']} {m['lang']} {m['engine_build']} {m['device_name']} {m['threads']} thr: RTF {m['rtf']:.3f}, commit {mib(m['peak_commit_bytes'])} MiB (the night model, behind the night pick)"
        )
    out.append("")
    dec = cap["decision"]
    out.append("The profile's pick: config " + json.dumps(dec["config"]["config"]) + f", cpu_threads {dec['config']['cpu_threads']}; " + "; ".join(
        f"{l}: rolling {v['rolling']['model']} RTF {v['rolling']['rtf']:.3f}" + (f", night {v['night']['model']} RTF {v['night']['rtf']:.3f}" if v.get("night") else "") for l, v in dec["languages"].items()
    ))
    out.append("")
    out.append("Where they differ (arithmetic from the files; the reading is in a note that names its author)")
    out.append("  device, model, threads and beam/best-of: the same as the cell in every unit")
    out.append("  what the cell does not do, and a unit does: language detection on every chunk (two languages), sentence split, one engine process")
    out.append("  per unit (load and first-use cost each time), the unit's own audio (a concatenation of FLEURS sentences with 0.5 to 3 s pauses)")
    text = "\n".join(out) + "\n"
    if a.out:
        Path(a.out).write_text(text, encoding="utf8")
    sys.stdout.reconfigure(encoding="utf8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
