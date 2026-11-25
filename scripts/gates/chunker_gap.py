"""Phase 5, item 5: the chunker's merge-gap cap, before and after, on the chunks `wb-engine chunks` gives.

    eval/.venv/Scripts/python scripts/gates/chunker_gap.py --before ENGINE.exe --after ENGINE.exe \
        --work D:/STELMAR-Gate/chunker-gap --out docs/evidence/phase-5/chunker-gap.txt

`chunks` runs the VAD and the chunker only (no model, no decoding). Two inputs:

  1. Synthetic audio: FLEURS English and Hindi utterances (never CoSHE) joined with short pauses and, between some of them,
     silences of 5, 10 and 15 s, to show what a long pause inside a chunk does: chunks under the low-speech bar of 0.3
     (PLAN section 6; the engine drops such a chunk) and the seconds of speech inside them.
  2. The first 20 CoSHE-Eval clips (the 20 of the engine gates): chunks per clip before and after, which must not change.
     Only counts are written; no text and no audio leave the machine.

  3. The 60 FLEURS clips of the Phase 2 parity gate (eval/clips/phase2-verify-fleurs.txt) and the day-simulation sessions
     (--day-audio): chunk boundaries before and after. Identical boundaries mean identical decodes, so the gates that
     decode those chunks cannot change.

Both engines are run on the same files with the same settings; both binaries' SHA-256 are in the evidence. Never writes
audio into the repository: the synthetic file lives under --work.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))

from wbeval.data import load_manifest  # noqa: E402

RATE = 16000
LOW_SPEECH = 0.3
# Silences (seconds) put between the synthetic utterances, in order and repeating; the short ones stay under the cap.
PAUSES = [0.8, 5.0, 0.8, 10.0, 1.5, 15.0, 0.8, 5.0, 10.0, 0.8]
PIECES_S = [1.2, 2.0]  # seconds taken from the start of each utterance, alternating: under the 3 s sentence minimum


def read_wav(path: Path) -> bytes:
    with wave.open(str(path)) as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2, path
        return w.readframes(w.getnframes())


def build_synthetic(out: Path, per_lang: int = 16) -> dict:
    """Short pieces of speech (the first 1.2 s or 2.0 s of FLEURS utterances) with silences between them. Until the cap, the
    chunker merged a piece shorter than the 3 s sentence minimum with the next one across any pause that still fit in the
    15 s limit, so 1.2 s of speech, 10 s of silence and 2.0 s of speech became one chunk with a speech share of 0.24."""
    langs = []
    for lang in ("en", "hi"):
        utts = sorted(load_manifest(f"fleurs-{lang}"), key=lambda u: u.id)[:per_lang]
        langs.append([str(REPO / "eval" / "data" / f"fleurs-{lang}" / u.audio) for u in utts])
    order = [w for pair in zip(*langs, strict=True) for w in pair]
    frames = b""
    gaps = []
    for i, wav in enumerate(order):
        n = int(RATE * PIECES_S[i % len(PIECES_S)])
        frames += read_wav(Path(wav))[: 2 * n]
        pause = PAUSES[i % len(PAUSES)]
        gaps.append(pause)
        frames += bytes(2) * int(RATE * pause)
    out.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(out), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(frames)
    return {"utterances": len(order), "seconds": round(len(frames) / 2 / RATE, 1), "pauses": gaps}


def run_chunks(engine: Path, wav: Path, out_json: Path) -> dict:
    p = subprocess.run(
        [str(engine), "chunks", "--file", str(wav), "--out", str(out_json)],
        capture_output=True, text=True, cwd=str(REPO), check=False,
    )
    if p.returncode != 0:
        raise RuntimeError(f"{engine.name} chunks failed on {wav.name}: exit {p.returncode}: {p.stderr[-300:]}")
    return json.loads(out_json.read_text(encoding="utf-8"))


def summarise(doc: dict) -> dict:
    cs = doc["chunks"]
    low = [c for c in cs if c["speech_ratio"] < LOW_SPEECH]
    lost_s = sum(c["speech_ratio"] * (c["end_ms"] - c["start_ms"]) / 1000.0 for c in low)
    longest = max(((c["end_ms"] - c["start_ms"]) / 1000.0 for c in cs), default=0.0)
    return {"chunks": len(cs), "low": len(low), "low_speech_s": round(lost_s, 1), "longest_s": round(longest, 1)}


def wav_seconds(path: Path) -> float:
    with wave.open(str(path)) as w:
        return w.getnframes() / w.getframerate()


def bounds(doc: dict) -> list[tuple[int, int]]:
    return [(c["start_ms"], c["end_ms"]) for c in doc["chunks"]]


def compare_set(L: list[str], title: str, files: list[Path], before: Path, after: Path, work: Path) -> None:
    same = 0
    tb = ta = lb = la = 0
    ls_b = ls_a = 0.0
    differing = []
    for f in files:
        db = run_chunks(before, f, work / "x-before.json")
        da = run_chunks(after, f, work / "x-after.json")
        sb, sa = summarise(db), summarise(da)
        tb += sb["chunks"]; ta += sa["chunks"]; lb += sb["low"]; la += sa["low"]
        ls_b += sb["low_speech_s"]; ls_a += sa["low_speech_s"]
        if bounds(db) == bounds(da):
            same += 1
        else:
            differing.append(f"{f.name}: {sb['chunks']} -> {sa['chunks']} chunks")
    L.append(f"== {title}")
    L.append(f"files {len(files)}; chunks before {tb}, after {ta}; chunks under the speech bar before {lb} ({ls_b:.1f} s of speech), after {la} ({ls_a:.1f} s); "
             f"files with identical chunk boundaries {same} of {len(files)}")
    for d in differing:
        L.append("  differs: " + d)
    L.append("")


def sha(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--before", required=True)
    ap.add_argument("--after", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out")
    ap.add_argument("--clips", type=int, default=20)
    ap.add_argument("--day-audio")
    a = ap.parse_args()
    before, after, work = Path(a.before), Path(a.after), Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    L: list[str] = []
    L.append("Chunker merge-gap cap, before and after (Phase 5, item 5). Written by scripts/gates/chunker_gap.py; nobody edits it.")
    L.append(f"before engine: {before.name} sha256 {sha(before)[:16]}")
    L.append(f"after engine:  {after.name} sha256 {sha(after)[:16]}")
    L.append("method: wb-engine chunks (VAD and chunker only, default settings: 15 s limit, sentence split, the cap of the after engine is 2 s)")
    L.append(f"bar: a chunk whose speech share is under {LOW_SPEECH} is dropped by the engine (PLAN section 6)")
    L.append("")
    syn = work / "synthetic-long-pauses.wav"
    info = build_synthetic(syn)
    L.append(f"== synthetic: the first 1.2 s or 2.0 s of FLEURS English and Hindi utterances, {info['utterances']} of them, {info['seconds']} s, pauses of 0.8 to 15 s")
    sb = summarise(run_chunks(before, syn, work / "syn-before.json"))
    sa = summarise(run_chunks(after, syn, work / "syn-after.json"))
    L.append("engine | chunks | chunks under the speech bar | seconds of speech in them | longest chunk s")
    L.append(f"before | {sb['chunks']} | {sb['low']} | {sb['low_speech_s']} | {sb['longest_s']}")
    L.append(f"after  | {sa['chunks']} | {sa['low']} | {sa['low_speech_s']} | {sa['longest_s']}")
    L.append("")
    utts = load_manifest("coshe-eval")[: a.clips]
    L.append(f"== CoSHE-Eval, the first {len(utts)} clips (counts only)")
    rows = []
    same = 0
    tb = ta = lb = la = 0
    for i, u in enumerate(utts):
        wav = REPO / "eval" / "data" / "coshe-eval" / u.audio
        db = run_chunks(before, wav, work / "c-before.json")
        da = run_chunks(after, wav, work / "c-after.json")
        cb, ca = summarise(db), summarise(da)
        same += int(bounds(db) == bounds(da))
        tb += cb["chunks"]; ta += ca["chunks"]; lb += cb["low"]; la += ca["low"]
        rows.append(f"clip {i + 1:02d} | {cb['chunks']} | {ca['chunks']} | {cb['low']} | {ca['low']}")
    L.append("clip | chunks before | chunks after | under the bar before | under the bar after")
    L += rows
    L.append(f"total | {tb} | {ta} | {lb} | {la}; clips with identical chunk boundaries: {same} of {len(utts)}")
    L.append("")
    pfiles = []
    for ln in (REPO / "eval" / "clips" / "phase2-verify-fleurs.txt").read_text(encoding="utf-8").splitlines():
        if ln.strip() and not ln.startswith("#") and "\t" in ln:
            ds, uid = ln.split("\t")[:2]
            m = {u.id: u for u in load_manifest(ds.strip())}
            pfiles.append(REPO / "eval" / "data" / ds.strip() / m[uid.strip()].audio)
    compare_set(L, "the 60 FLEURS clips of the Phase 2 parity gate", pfiles, before, after, work)
    if a.day_audio:
        days = sorted(Path(a.day_audio).glob("*.wav"))
        compare_set(L, f"day-simulation sessions ({len(days)} files, {sum(wav_seconds(f) for f in days) / 60:.0f} min)", days, before, after, work)
    text = "\n".join(L) + "\n"
    if a.out:
        out = Path(a.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text, encoding="utf-8", newline="\n")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
