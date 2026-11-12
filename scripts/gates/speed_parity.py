"""Phase 2: wb-engine against whisper-cli on speed, timed the same way (not a WER test).

    eval/.venv/Scripts/python scripts/gates/speed_parity.py --out docs/evidence/phase-2/speed-parity.txt

What is compared: the same clips (single-chunk FLEURS utterances from the committed clip list: one VAD chunk,
so both sides do one encoder pass and one decode), the same model file, the same settings
(`eval/roles.toml` rolling: beam 5, best-of 5, temperature fallback on, no timestamps, no previous-text
context), the same thread count, the same whisper.cpp v1.9.5, both builds portable AVX2 (GGML_NATIVE=OFF).
The whisper-cli command is the harness's own (`WhisperCli._args`); the engine command is the harness's
(`WbEngine.command`). Runs alternate A, B, A, B so a drifting machine hits both sides equally; one
unrecorded warm-up per side. Each process is a fresh one, so page-cache effects are the same.

Reported separately, per side: model load, processing (everything after the load), the whole process's
wall time. whisper-cli's numbers come from its own timing lines on stderr; the engine's from its `ready`
(`load_s`) and `stats` (`decode_s`, which includes its VAD) events. The VAD of the engine is timed on
its own with `wb-engine chunks`, because whisper-cli in this comparison gets the clip without VAD.

Timing-sensitive: holds the gate lock and the busy lock like every timed run (`run_heavy`), refuses to start
while an engine build runs, and writes "overlay tools closed". Development time only.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

import run as harness  # noqa: E402
from wbeval import manifest  # noqa: E402
from wbeval.common import run_heavy  # noqa: E402
from wbeval.engines import WbEngine, WhisperCli  # noqa: E402
from wbeval.tools import whisper_cpp_info  # noqa: E402

BUILD_PROCESSES = {"cargo.exe", "rustc.exe", "cl.exe", "link.exe", "cmake.exe", "ninja.exe", "nvcc.exe", "glslc.exe", "MSBuild.exe"}


def refuse_if_building() -> None:
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True).stdout
    running = sorted({line.split('","')[0].strip('"') for line in out.splitlines()} & BUILD_PROCESSES)
    if running or Path(r"D:\Projects\stelmar-build.lock").exists():
        raise SystemExit(f"an engine build is running ({', '.join(running) or 'build lock present'}); timing runs never overlap builds")
    for tool in ("RTSS.exe", "MSIAfterburner.exe"):
        if tool in out:
            raise SystemExit(f"{tool} is running; close the overlay tools first")


def timing_ms(stderr: str, name: str) -> float | None:
    m = re.search(rf"{name} time =\s*([\d.]+) ms", stderr)
    return float(m.group(1)) if m else None


def run_cli(cli: WhisperCli, model: str, settings, wav: Path, flash: bool, lang: str) -> dict:
    # The harness passes -np (print results only), which also hides the timing lines; it changes nothing in the
    # decoding, so it is dropped here to read the model load and total times.
    cmd = [a for a in cli._args(model, settings) if a != "-np"] + ([] if flash else ["-nfa"]) + ["-l", lang, "-f", str(wav)]
    t0 = time.perf_counter()
    proc = run_heavy(cmd, "speed parity: whisper-cli", capture_output=True, text=True, encoding="utf-8", errors="replace")
    wall = time.perf_counter() - t0 - proc.gate_wait_s
    if proc.returncode != 0:
        raise RuntimeError(f"whisper-cli failed: {proc.stderr[-400:]}")
    load = (timing_ms(proc.stderr, "load") or 0) / 1000
    total = (timing_ms(proc.stderr, "total") or 0) / 1000
    return {"load": load, "process": max(total - load, 0.0), "wall": wall, "text": " ".join(proc.stdout.split())}


def run_engine(eng: WbEngine, setup, lang: str, settings, wav: Path, flash: bool) -> dict:
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "segments.jsonl"
        cmd = eng.command(wav, setup, [lang], settings, out)
        i = cmd.index("--settings") + 1
        cmd[i] = json.dumps({**json.loads(cmd[i]), "flash_attn": flash}, separators=(",", ":"))
        t0 = time.perf_counter()
        proc = run_heavy(cmd, "speed parity: wb-engine", capture_output=True, text=True, encoding="utf-8", errors="replace")
        wall = time.perf_counter() - t0 - proc.gate_wait_s
        if proc.returncode != 0:
            raise RuntimeError(f"wb-engine failed: {proc.stderr[-400:]}")
        events = [json.loads(x) for x in proc.stdout.splitlines() if x.strip().startswith("{")]
        text = " ".join(" ".join(json.loads(x)["text"].split()) for x in out.read_text(encoding="utf-8").splitlines()
                        if x.strip() and json.loads(x).get("event") == "segment")
    load = next(e["load_s"] for e in events if e.get("event") == "ready")
    stats = [e for e in events if e.get("event") == "stats"]
    chunks = sum(e["chunks"] for e in stats)
    return {"load": load, "process": sum(e["decode_s"] for e in stats), "wall": wall, "chunks": chunks, "text": text}


def engine_vad_s(binary: Path, wav: Path) -> float:
    with tempfile.TemporaryDirectory() as tmp:
        t0 = time.perf_counter()
        proc = run_heavy([str(binary), "chunks", "--file", str(wav), "--out", str(Path(tmp) / "c.json")], "speed parity: vad",
                         capture_output=True, text=True)
        wall = time.perf_counter() - t0 - proc.gate_wait_s
        if proc.returncode != 0:
            raise RuntimeError(f"wb-engine chunks failed: {proc.stderr[-300:]}")
        return wall


def word_diff(a: str, b: str) -> int:
    """Word-level edit distance between two texts (0 = identical words)."""
    x, y = a.split(), b.split()
    prev = list(range(len(y) + 1))
    for i, wx in enumerate(x, 1):
        cur = [i]
        for j, wy in enumerate(y, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (wx != wy)))
        prev = cur
    return prev[-1]


def med(xs: list[float]) -> float:
    return statistics.median(xs)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--model", default="medium")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--clips", type=int, default=3, help="clips per language")
    ap.add_argument("--langs", default="en,hi")
    ap.add_argument("--repeats", type=int, default=3)
    ap.add_argument("--engine", default=str(REPO / "src-tauri" / "binaries" / "wb-engine-avx2-x86_64-pc-windows-msvc.exe"))
    ap.add_argument("--flash-attn", choices=("on", "off"), default="on")
    args = ap.parse_args()

    refuse_if_building()
    flash = args.flash_attn == "on"
    binary = Path(args.engine)
    eng = WbEngine(binary=binary)
    cli = WhisperCli()
    setup = harness.parse_setup(args.model, {})
    settings = harness.role_settings("rolling", args.threads, None)
    settings.max_chunk_s = 15.0
    wcpp = whisper_cpp_info()
    ver = subprocess.run([str(binary), "--version"], capture_output=True, text=True).stdout.strip()
    lines = [
        "Speed parity: wb-engine against whisper-cli (Phase 2)",
        f"date: {dt.datetime.now().strftime('%Y-%m-%d %H:%M')} IST; machine: Legion native (no emulation), {args.threads} threads",
        "overlay tools closed: yes; no engine build running (checked at start); gate lock and busy lock held per process",
        f"whisper-cli: whisper.cpp {wcpp.get('tag')} commit {wcpp.get('commit')} (OpenMP on, flash attention default on)",
        f"wb-engine: {ver} (avx2, GGML_NATIVE=OFF, no OpenMP, flash attention {args.flash_attn})",
        f"model: {args.model} f16, same file; settings: beam {settings.beam_size}, best-of {settings.best_of}, temperature fallback "
        f"{settings.temperature_fallback}, no timestamps, no previous-text context for a single chunk; runs alternate cli/engine; 1 warm-up per side then {args.repeats} each",
        f"clips: single-chunk FLEURS utterances from eval/clips/phase2-verify-fleurs.txt (sha256 {harness.limit_key('eval/clips/phase2-verify-fleurs.txt')[6:20]}...)",
        "",
    ]
    rows = []
    for lang in args.langs.split(","):
        ds = f"fleurs-{lang}"
        ids = harness.clip_list_ids("eval/clips/phase2-verify-fleurs.txt", ds)
        utts = {u.id: u for u in harness.load_manifest(ds)}
        picked = []
        for uid in ids:
            u = utts[uid]
            wav = REPO / "eval" / "data" / ds / u.audio
            if 6 <= u.duration_s <= 14:
                picked.append((uid, wav, u.duration_s))
            if len(picked) == args.clips:
                break
        for uid, wav, dur in picked:
            # one warm-up per side
            run_cli(cli, args.model, settings, wav, flash, lang)
            run_engine(eng, setup, lang, settings, wav, flash)
            c, e = [], []
            for _ in range(args.repeats):
                c.append(run_cli(cli, args.model, settings, wav, flash, lang))
                e.append(run_engine(eng, setup, lang, settings, wav, flash))
            vad_s = med([engine_vad_s(binary, wav) for _ in range(args.repeats)])
            rows.append((ds, uid, dur, c, e, vad_s))
    lines.append("clip | audio s | side | model load s | processing s | process wall s | chunks | text against whisper-cli")
    gaps = []
    for ds, uid, dur, c, e, vad_s in rows:
        d = word_diff(c[0]["text"], e[0]["text"])
        same = "identical" if d == 0 else f"{d} of {len(c[0]['text'].split())} words differ"
        if d:
            with open(Path(tempfile.gettempdir()) / "speed-parity-diffs.txt", "a", encoding="utf-8") as f:
                f.write("\n".join([f"{ds}/{uid}", f" cli: {c[0]['text']}", f" eng: {e[0]['text']}", ""]))
        lines.append(f"{ds}/{uid[:8]} | {dur:.2f} | whisper-cli | {med([x['load'] for x in c]):.2f} | {med([x['process'] for x in c]):.2f} | {med([x['wall'] for x in c]):.2f} | 1 | {same}")
        lines.append(f"{ds}/{uid[:8]} | {dur:.2f} | wb-engine   | {med([x['load'] for x in e]):.2f} | {med([x['process'] for x in e]):.2f} | {med([x['wall'] for x in e]):.2f} | {e[0].get('chunks')} | (engine VAD alone: {vad_s:.2f} s wall incl. start-up)")
        gaps.append((med([x["process"] for x in e]) / med([x["process"] for x in c]) - 1, med([x["load"] for x in e]) / med([x["load"] for x in c]) - 1,
                     med([x["wall"] for x in e]) / med([x["wall"] for x in c]) - 1))
    if gaps:
        lines += ["", "wb-engine relative to whisper-cli (positive = engine slower), median over clips:",
                  f"  processing: {statistics.median(g[0] for g in gaps) * 100:+.1f}%   model load: {statistics.median(g[1] for g in gaps) * 100:+.1f}%   whole process: {statistics.median(g[2] for g in gaps) * 100:+.1f}%"]
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
