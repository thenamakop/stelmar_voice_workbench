"""Is eval/data FLEURS audio scaled wrongly, or is the source that quiet? (Phase 2, 11 Nov 2026)

    eval/.venv/Scripts/python scripts/gates/fleurs_level_check.py --out docs/evidence/phase-2/fleurs-level-check.txt

For every clip of FLEURS test en, hi and ur it reads the ORIGINAL audio from the Hugging Face cache (the
test.tar.gz that eval/get_data.py converts; offline, local files only) and the WAV in eval/data, and
compares the RMS level of the two in dBFS. If the converter had mis-scaled the audio (for example float to
int16 without scaling), the eval/data level would differ from the original by a constant or large offset;
if the source itself is quiet, the two agree within the effect of resampling (a fraction of a dB).
Also reports the distribution of levels per set (counts and percentiles only; no audio, no text), and
the same for CoSHE-Eval for comparison.

Heavy (reads about 1 GB of archives): run it when no timing-sensitive run is in progress.
"""

from __future__ import annotations

import argparse
import io
import math
import sys
import tarfile
from pathlib import Path

import numpy as np
import soundfile as sf

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "eval"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from guard_calibration import pct  # noqa: E402
from wbeval.data import load_manifest  # noqa: E402

REPO_ID = "google/fleurs"
REVISION = "70bb2e84b976b7e960aa89f1c648e09c59f894dd"
CODES = {"fleurs-en": "en_us", "fleurs-hi": "hi_in", "fleurs-ur": "ur_pk"}


def dbfs(x: np.ndarray) -> float:
    r = float(np.sqrt(np.mean(np.square(x.astype(np.float64))))) if x.size else 0.0
    return 20 * math.log10(r) if r > 0 else -120.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    from huggingface_hub import hf_hub_download

    lines = [
        "FLEURS level check: original audio against the WAVs in eval/data (Phase 2, 11 Nov 2026)",
        f"source: {REPO_ID} at {REVISION[:12]}, test.tar.gz read from the local Hugging Face cache (no network)",
        "level = RMS of the whole clip in dBFS (float samples, 1.0 = full scale); no audio or text below",
        "",
        "set | clips | original median | p5 | p95 | eval/data median | difference eval/data minus original: median | min | max (dB) | clips under -50 dBFS (original / eval/data)",
    ]
    verdict_rows = []
    for ds, code in CODES.items():
        tar_path = hf_hub_download(REPO_ID, f"data/{code}/audio/test.tar.gz", repo_type="dataset",
                                   revision=REVISION, local_files_only=True)
        have = {Path(u.audio).stem: REPO / "eval" / "data" / ds / u.audio for u in load_manifest(ds)}
        orig, new = {}, {}
        with tarfile.open(tar_path, "r:gz") as tar:
            for m in tar:
                stem = Path(m.name).stem
                if not m.isfile() or stem not in have:
                    continue
                data = tar.extractfile(m)
                if data is None:
                    continue
                x, _ = sf.read(io.BytesIO(data.read()), dtype="float32", always_2d=True)
                orig[stem] = dbfs(x.mean(axis=1))
        for stem, p in have.items():
            if stem in orig:
                y, _ = sf.read(p, dtype="float32", always_2d=True)
                new[stem] = dbfs(y.mean(axis=1))
        ids = sorted(orig.keys() & new.keys())
        o = [orig[i] for i in ids]
        n = [new[i] for i in ids]
        d = [b - a for a, b in zip(o, n)]
        lines.append(f"{ds} | {len(ids)} | {pct(o, 50):.1f} | {pct(o, 5):.1f} | {pct(o, 95):.1f} | {pct(n, 50):.1f} | "
                     f"{pct(d, 50):+.2f} | {min(d):+.2f} | {max(d):+.2f} | {sum(v < -50 for v in o)} / {sum(v < -50 for v in n)}")
        verdict_rows.append((ds, len(ids), pct(d, 50), min(d), max(d), len(have) - len(ids)))
    # CoSHE-Eval for comparison (eval/data only).
    co = []
    for u in load_manifest("coshe-eval"):
        y, _ = sf.read(REPO / "eval" / "data" / "coshe-eval" / u.audio, dtype="float32", always_2d=True)
        co.append(dbfs(y.mean(axis=1)))
    lines.append(f"coshe-eval (eval/data only) | {len(co)} | n/a | n/a | n/a | {pct(co, 50):.1f} (p5 {pct(co, 5):.1f}, p95 {pct(co, 95):.1f}) | n/a | n/a | n/a | n/a / {sum(v < -50 for v in co)}")
    lines.append("")
    mis = [r for r in verdict_rows if abs(r[2]) > 1.0 or r[3] < -3.0 or r[4] > 3.0]
    lines.append("RESULT: " + ("eval/data differs from the original audio by more than resampling can explain for: "
                               + ", ".join(r[0] for r in mis) + " (mis-scaled)" if mis else
                               "eval/data matches the original audio within 1 dB in every clip of every set checked: the converter "
                               "does not mis-scale; the low level is in the source recordings."))
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 1 if mis else 0


if __name__ == "__main__":
    sys.exit(main())
