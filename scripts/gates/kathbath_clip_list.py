"""Write the fixed-seed Kathbath test-unknown clip lists of the Phase 3 turbo-vs-medium comparison.

Picks N utterances whose id starts with `tu_` (the official test-unknown split) from
eval/data/kathbath-<lang>/manifest.jsonl with random.Random(seed).sample over the sorted ids, and writes
eval/clips/phase3-kathbath-<lang>.txt in the format of phase2-verify-fleurs.txt (dataset, TAB, id; ids only,
no audio and no text). Run with the eval environment:
    eval/.venv/Scripts/python scripts/gates/kathbath_clip_list.py
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SEED = 20261113
COUNT = 100


def pick(ids: list[str], count: int, seed: int) -> list[str]:
    pool = sorted(i for i in ids if i.startswith("tu_"))
    if len(pool) < count:
        raise SystemExit(f"only {len(pool)} test-unknown utterances, need {count}")
    return sorted(random.Random(seed).sample(pool, count))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--count", type=int, default=COUNT)
    parser.add_argument("--langs", default="hi,ur")
    args = parser.parse_args()
    for lang in args.langs.split(","):
        manifest = REPO / "eval" / "data" / f"kathbath-{lang}" / "manifest.jsonl"
        ids = [json.loads(line)["id"] for line in manifest.read_text(encoding="utf-8").splitlines() if line.strip()]
        chosen = pick(ids, args.count, args.seed)
        digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
        out = REPO / "eval" / "clips" / f"phase3-kathbath-{lang}.txt"
        lines = [
            f"# Phase 3 turbo-vs-medium comparison: {args.count} Kathbath {lang} test-unknown utterances (id prefix tu_),",
            f"# random.Random({args.seed}).sample over the sorted tu_ ids ({sum(i.startswith('tu_') for i in ids)} available);",
            f"# manifest sha256 {digest}. Written by scripts/gates/kathbath_clip_list.py.",
            "# dataset<TAB>utterance id. Ids only: no audio, no text.",
        ] + [f"kathbath-{lang}\t{i}" for i in chosen]
        out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        print(f"{out.relative_to(REPO)}: {len(chosen)} ids")
    return 0


if __name__ == "__main__":
    sys.exit(main())
