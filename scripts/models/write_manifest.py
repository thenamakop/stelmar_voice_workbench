"""Writes models/manifest.json (committed, metadata only) from models/manifest.local.json (git-ignored).

    eval/.venv/Scripts/python scripts/models/write_manifest.py [--check]

Keeps, per model: id, file (relative to models/), sha256, bytes, kind, base, quant, languages,
candidate_roles, validated_roles, provisional_rank (Phase 3, provisional until Phase 5), licence, source {repo, revision} and, for converted or quantised
files, conversion {script, settings, input_sha256, whisper_cpp_commit, deterministic}. Drops notes that
could hold local paths, candidates that were dropped, and every other field. `--check` only verifies that
the committed file matches what would be written. Development time only; the app never runs this.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "eval"))

from wbeval import manifest  # noqa: E402
from wbeval.common import MODELS_DIR, REPO  # noqa: E402

OUT = MODELS_DIR / "manifest.json"
KEEP = ("id", "file", "sha256", "bytes", "kind", "base", "quant", "languages", "candidate_roles",
        "validated_roles", "provisional_rank", "licence")


def parse_source(text: str | dict) -> dict:
    if isinstance(text, dict):
        return {"repo": text.get("repo", ""), "revision": text.get("revision", "")}
    head = re.split(r"[ (]", text.strip(), maxsplit=1)[0]
    head = head.removeprefix("https://huggingface.co/")
    repo, _, rev = head.partition("@")
    return {"repo": repo, "revision": rev}


def entry_for(m: dict) -> dict:
    out = {k: m[k] for k in KEEP if k in m}
    out["source"] = parse_source(m.get("source", ""))
    conv = m.get("conversion")
    if conv:
        out["conversion"] = {k: conv.get(k) for k in ("script", "settings", "input_sha256", "whisper_cpp_commit",
                                                      "deterministic")}
    return out


def build() -> dict:
    local = manifest.load()
    return {"version": 2,
            "_note": "Metadata only: ids, files, sizes, SHA-256, licences, sources and how a file was made. "
                     "The model files are never committed and never downloaded by the app; it refuses a file "
                     "whose SHA-256 differs from this list. Written by scripts/models/write_manifest.py.",
            "models": sorted((entry_for(m) for m in local["models"]), key=lambda e: e["id"])}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(build(), ensure_ascii=False, indent=2) + "\n"
    if args.check:
        return 0 if OUT.exists() and OUT.read_text(encoding="utf-8") == text else 1
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(REPO).as_posix()} ({len(build()['models'])} models)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
