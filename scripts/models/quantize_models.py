"""Quantise converted ggml models to q8_0 and q5_0 with whisper.cpp's whisper-quantize.

    eval/.venv/Scripts/python scripts/models/quantize_models.py                  # every converted model
    eval/.venv/Scripts/python scripts/models/quantize_models.py hindi-small --types q8_0
    eval/.venv/Scripts/python scripts/models/quantize_models.py --delete-f16     # when disk runs short

Reads models/converted/<id>.convert.json (written by convert_hf_whisper.py) and writes one entry per
output to models/manifest.local.json: id, file, SHA-256, bytes, base, quantisation, mel bins,
languages, candidate roles, licence, source and whisper.cpp version. Phase 5 ships q5_0 only when
its WER stays within 1 point of q8_0.

Development time only. Waits for the Track A gate lock and runs at below-normal priority.
"""

from __future__ import annotations

import argparse
import json
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "eval"))

from wbeval import manifest  # noqa: E402
from wbeval.common import (  # noqa: E402
    CONVERTED_DIR, MIN_FREE_GB, MIN_FREE_GB_LARGE, rel, run_heavy, setup_logging, sha256_file,
)
from wbeval.tools import whisper_cpp_info, whisper_quantize  # noqa: E402

log = logging.getLogger("quantize")
TYPES = ("q8_0", "q5_0")


def quantize(record: dict, qtype: str, force: bool) -> None:
    src = CONVERTED_DIR / record["file"]
    out = CONVERTED_DIR / f"ggml-{record['id']}-{qtype}.bin"
    if force or not out.exists():
        if not src.exists():
            raise FileNotFoundError(f"{rel(src)} is gone (deleted with --delete-f16?); convert again with --force")
        tmp = out.with_suffix(".tmp")
        log_path = CONVERTED_DIR / f"{record['id']}-{qtype}.quantize.log"
        with log_path.open("w", encoding="utf-8") as logf:
            proc = run_heavy([whisper_quantize(), src, tmp, qtype], f"quantising {record['id']} to {qtype}",
                             stdout=logf, stderr=logf,
                             min_free_gb=MIN_FREE_GB_LARGE if record["base"].startswith("large") else MIN_FREE_GB)
        if proc.returncode != 0:
            tmp.unlink(missing_ok=True)
            raise RuntimeError(f"whisper-quantize failed for {record['id']} {qtype}; see {rel(log_path)}")
        tmp.replace(out)
    wcpp = whisper_cpp_info()
    entry = {
        "id": f"{record['id']}-{qtype}", "candidate": record["id"], "file": f"converted/{out.name}",
        "sha256": sha256_file(out), "bytes": out.stat().st_size, "kind": "whisper", "base": record["base"],
        "quant": qtype, "n_mels": record["inspection"]["n_mels"], "languages": record["languages"],
        "candidate_roles": record["candidate_roles"], "validated_roles": [], "licence": record["licence"],
        "licence_evidence": record["licence_evidence"], "source": record["source"],
        "converted_from_sha256": record["sha256"],
        "not_independent_on": record.get("not_independent_on", []),
        "independence_unknown_on": record.get("independence_unknown_on", []),
        "training_data_evidence": record.get("training_data_evidence", ""),
        "whisper_cpp": f"{wcpp['tag']} ({wcpp['commit'][:12]})",
        "notes": "; ".join(filter(None, [record.get("notes", ""), *record["inspection"]["problems"],
                                         *record["inspection"]["notes"]])),
    }
    manifest.upsert(entry)
    log.info("%s: %s %.2f GB sha256 %s", entry["id"], rel(out), entry["bytes"] / 1e9, entry["sha256"][:16])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ids", nargs="*", help="candidate ids (default: every converted model)")
    parser.add_argument("--types", nargs="+", default=list(TYPES), choices=TYPES)
    parser.add_argument("--force", action="store_true", help="quantise again even if the output exists")
    parser.add_argument("--delete-f16", action="store_true",
                        help="delete the f16 intermediate after both quantisations succeed")
    args = parser.parse_args()
    setup_logging("quantize")
    records = {p.name.removesuffix(".convert.json"): json.loads(p.read_text(encoding="utf-8"))
               for p in sorted(CONVERTED_DIR.glob("*.convert.json"))}
    ids = args.ids or sorted(records)
    failed = 0
    for cid in ids:
        if cid not in records:
            log.error("%s has not been converted (no %s.convert.json)", cid, cid)
            failed += 1
            continue
        try:
            for qtype in args.types:
                quantize(records[cid], qtype, args.force)
            if args.delete_f16 and all((CONVERTED_DIR / f"ggml-{cid}-{t}.bin").exists() for t in TYPES):
                f16 = CONVERTED_DIR / records[cid]["file"]
                if f16.exists():
                    f16.unlink()
                    manifest.remove(f"{cid}-f16")
                    log.info("%s: deleted %s", cid, rel(f16))
        except Exception:
            log.exception("%s failed", cid)
            failed += 1
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
