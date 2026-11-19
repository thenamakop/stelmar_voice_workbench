"""Upstream-break analysis of a reference capture against a SPEECH source (day simulation, 19 Nov 2026).

wb-testbench `analyse` regenerates a known noise signal from a seed, so it cannot judge a source made of speech. This
script does the same job for any source WAV: it finds where the source lies in the reference capture, then follows it
block by block (one second of the source at a time) and reports where the alignment jumped or the audio did not match.
It writes reference-analysis.json in the schema Get-WbUpstreamBreaks / Get-WbRunVerdict read
(scripts/lib/Fidelity.psm1), so the Phase 1 recording rule applies unchanged.

    python -I speech_analyse.py --reference <reference session dir> --source <source.wav> --out <reference-analysis.json>

Method (all in channel 0 at 8 kHz, a decimation by 6 of the 48 kHz audio; positions reported in 48 kHz frames):
  1. The first loud second of the source is searched in the first minute of the reference (normalised cross-correlation);
     that gives the alignment offset (reference position minus source position).
  2. Every later block with speech (RMS above a floor) is matched near the expected position, +-0.4 s (then +-2.0 s if
     that fails). A peak at or above MATCH is accepted. When the peak is more than JUMP_MS from the expected offset,
     that is a discontinuity: source frames skipped (the offset fell) or repeated (it rose). A block with no peak at or
     above MATCH is an unmatched stretch. Silent blocks are skipped (no alignment can be read in silence), so a break
     inside silence shows at the next block with speech.
Breaks shorter than JUMP_MS (1 ms) are not reported; the cable path is bit-exact in the Phase 1 gates, so a real break
is several milliseconds or more.
"""

from __future__ import annotations

import argparse
import json
import sys
import wave
from pathlib import Path

import numpy as np

DEC = 6  # 48 kHz -> 8 kHz
BLOCK = 8000  # one second at 8 kHz
NEAR = 3200  # +-0.4 s
FAR = 16000  # +-2.0 s
MATCH = 0.6
JUMP_MS = 1.0
SILENT_RMS = 150.0  # 16-bit units; speech is far above, the digital silence between utterances is 0


def read_channel0(path: Path) -> np.ndarray:
    """Channel 0 of a 16-bit WAV, decimated by DEC with an average (a crude low pass), as float32."""
    w = wave.open(str(path), "rb")
    ch, width, n = w.getnchannels(), w.getsampwidth(), w.getnframes()
    if width != 2:
        raise SystemExit(f"{path}: only 16-bit WAV is supported")
    out = np.empty(n // DEC, dtype=np.float32)
    step = DEC * 48000  # frames per read: one second times DEC keeps the reshape exact
    pos = 0
    while pos < len(out):
        take = min(step // DEC, len(out) - pos)
        raw = np.frombuffer(w.readframes(take * DEC), dtype="<i2")
        raw = raw[: (len(raw) // ch) * ch].reshape(-1, ch)[:, 0].astype(np.float32)
        raw = raw[: (len(raw) // DEC) * DEC].reshape(-1, DEC).mean(axis=1)
        out[pos : pos + len(raw)] = raw
        pos += len(raw)
        if len(raw) == 0:
            break
    w.close()
    return out[:pos]


def read_reference(session: Path) -> tuple[np.ndarray, int, int]:
    parts = json.loads((session / "parts.json").read_text(encoding="utf-8-sig"))["parts"]
    chunks = [read_channel0(session / p["file"]) for p in sorted(parts, key=lambda p: p["part"])]
    frames = sum(int(p["frames"]) for p in parts)
    return np.concatenate(chunks), frames, len(parts)


def ncc(block: np.ndarray, window: np.ndarray) -> tuple[int, float]:
    """Best lag k (0..len(window)-len(block)) of `block` inside `window` and its normalised correlation."""
    n = len(block)
    m = len(window)
    if m < n:
        return 0, 0.0
    size = 1 << int(np.ceil(np.log2(m + n)))
    fb = np.fft.rfft(block[::-1], size)
    fw = np.fft.rfft(window, size)
    num = np.fft.irfft(fb * fw, size)[n - 1 : m]  # lags 0 .. m-n
    csum = np.concatenate([[0.0], np.cumsum(window.astype(np.float64) ** 2)])
    wn = np.sqrt(np.maximum(csum[n:] - csum[: m - n + 1], 1e-9))
    bn = float(np.sqrt(np.sum(block.astype(np.float64) ** 2))) or 1e-9
    # A window much quieter than the block cannot be the block (and its norm is lost in the cumulative sum's rounding).
    r = np.where(wn > 0.1 * bn, num / (wn * bn), 0.0)
    k = int(np.argmax(r))
    return k, float(r[k])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--reference", required=True)
    ap.add_argument("--source", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    session = Path(a.reference)
    src = read_channel0(Path(a.source))
    ref, ref_frames, nparts = read_reference(session)
    with wave.open(a.source, "rb") as w:
        src_frames, rate = w.getnframes(), w.getframerate()
    rms = lambda x: float(np.sqrt(np.mean(x.astype(np.float64) ** 2)))  # noqa: E731

    nblocks = len(src) // BLOCK
    loud = [rms(src[i * BLOCK : (i + 1) * BLOCK]) >= SILENT_RMS for i in range(nblocks)]
    if not any(loud):
        raise SystemExit("the source has no speech")

    # 1. the first alignment: the first loud block, searched in the first minute (and a little more) of the reference.
    first = loud.index(True)
    head = ref[: min(len(ref), 60 * 8000 + first * BLOCK + BLOCK)]
    k0, c0 = ncc(src[first * BLOCK : (first + 1) * BLOCK], head)
    if c0 < MATCH:
        raise SystemExit(f"the source was not found in the reference (best correlation {c0:.2f})")
    offset = k0 - first * BLOCK  # reference position minus source position, in 8 kHz samples

    discontinuities, unmatched, correlations = [], [], []
    last_matched_end = None
    for i in range(first, nblocks):
        if not loud[i]:
            continue
        s = src[i * BLOCK : (i + 1) * BLOCK]
        exp = i * BLOCK + offset
        found = None
        for width in (NEAR, FAR):
            lo = max(0, exp - width)
            hi = min(len(ref), exp + BLOCK + width)
            if hi - lo < BLOCK:
                break
            k, c = ncc(s, ref[lo:hi])
            if c >= MATCH:
                found = (lo + k - i * BLOCK, c)
                break
        if found is None:
            src_ms = i * BLOCK / 8.0
            if unmatched and abs(src_ms - (unmatched[-1]["source_ms"] + unmatched[-1]["ms"])) < 1.0:
                unmatched[-1]["ms"] += 1000.0
            else:
                unmatched.append({"source_ms": src_ms, "ms": 1000.0})
            continue
        new_offset, c = found
        correlations.append(c)
        shift = new_offset - offset  # samples at 8 kHz; positive: the reference moved on (source frames repeated)
        if abs(shift) * 1000.0 / 8000.0 >= JUMP_MS:
            frames = -shift * DEC  # source frames skipped (positive) or repeated (negative)
            discontinuities.append(
                {
                    "at_ms": (i * BLOCK + new_offset) / 8.0,
                    "source_ms": i * BLOCK / 8.0,
                    "part": 1,
                    "source_jump_frames": int(frames),
                    "source_jump_ms": frames * 1000.0 / rate,
                }
            )
        offset = new_offset
        last_matched_end = (i + 1) * BLOCK + offset

    first_offset_ms = (k0 - first * BLOCK) / 8.0
    content_start_ms = max(0.0, first_offset_ms)
    source_ms = src_frames * 1000.0 / rate
    last_ms = (last_matched_end / 8.0) if last_matched_end is not None else content_start_ms
    xr_path = session / "xruns.json"
    xruns = json.loads(xr_path.read_text(encoding="utf-8-sig")) if xr_path.exists() else []
    unmatched_ms = float(sum(u["ms"] for u in unmatched))
    result = {
        "sample_rate": rate,
        "recorded_channels": 2,
        "parts": nparts,
        "recorded_ms": ref_frames * 1000.0 / rate,
        "source_ms": source_ms,
        "leading_junk_ms": 0.0,
        "content_start_ms": content_start_ms,
        "content_first_source_frame": 0,
        "content_ms": max(0.0, last_ms - content_start_ms),
        "duration_diff_ms": (last_ms - content_start_ms) - source_ms,
        "discontinuities": discontinuities,
        "silent_gap_ms": 0.0,
        "unmatched_ms": unmatched_ms,
        "unmatched_ranges": unmatched,
        "min_block_correlation": min(correlations) if correlations else 0.0,
        "passed": not discontinuities and unmatched_ms == 0.0,
        "method": "speech_analyse.py (block-wise alignment of a speech source in the reference capture; scripts/gates/speech_analyse.py)",
        "blocks_with_speech": int(sum(loud)),
        "blocks_matched": len(correlations),
    }
    if xruns:
        result["xruns"] = xruns
    Path(a.out).write_text(json.dumps(result, indent=1), encoding="utf-8")
    print(
        f"{len(correlations)} of {sum(loud)} blocks with speech matched, {len(discontinuities)} discontinuities, "
        f"{unmatched_ms / 1000:.0f} s unmatched, source starts {content_start_ms:.1f} ms into the reference"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
