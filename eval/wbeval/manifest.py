"""models/manifest.local.json: every converted, quantised or downloaded ggml file on this machine.

Git-ignored. Same field names as models/manifest.example.json plus provenance (n_mels, source
revision, whisper.cpp version, notes). Phase 5 copies validated entries into models/manifest.json.
"""

from __future__ import annotations

import datetime as dt
import json
from pathlib import Path

import tomllib

from .common import MODELS_DIR, REPO

MANIFEST = MODELS_DIR / "manifest.local.json"
CANDIDATES = REPO / "scripts" / "models" / "candidates.toml"


def load() -> dict:
    if MANIFEST.exists():
        return json.loads(MANIFEST.read_text(encoding="utf-8"))
    return {
        "version": 2,
        "_note": "Local, git-ignored. Written by scripts/models/*.py. Paths are relative to models/.",
        "models": [],
        "dropped": [],
    }


def save(manifest: dict) -> None:
    manifest["models"].sort(key=lambda m: m["id"])
    manifest["updated_at"] = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def upsert(entry: dict) -> None:
    manifest = load()
    manifest["models"] = [m for m in manifest["models"] if m["id"] != entry["id"]] + [entry]
    manifest["dropped"] = [d for d in manifest.get("dropped", []) if d["id"] != entry.get("candidate", entry["id"])]
    save(manifest)


def remove(model_id: str) -> None:
    manifest = load()
    manifest["models"] = [m for m in manifest["models"] if m["id"] != model_id]
    save(manifest)


def drop(candidate: str, reason: str, source: str = "") -> None:
    manifest = load()
    manifest["models"] = [m for m in manifest["models"] if m.get("candidate") != candidate]
    manifest["dropped"] = [d for d in manifest.get("dropped", []) if d["id"] != candidate] + [
        {"id": candidate, "reason": reason, "source": source,
         "date": dt.date.today().isoformat()}
    ]
    save(manifest)


def model_path(entry: dict) -> Path:
    return MODELS_DIR / entry["file"]


def find(model_id: str) -> dict:
    for m in load()["models"]:
        if m["id"] == model_id:
            return m
    raise KeyError(f"model {model_id!r} is not in {MANIFEST.name}; convert or download it first")


def training_flags(model_id: str) -> dict[str, list[str]]:
    """Evaluation sets a model must not be ranked on.

    `not_independent_on`: the model (or its corpus) was trained on this set.
    `independence_unknown_on`: its sources do not say whether the set was used for training.
    scripts/models/candidates.toml (committed) is authoritative, looked up through the manifest entry's
    `candidate` id; the entry's own fields are the fallback for entries without a candidate."""
    try:
        entry = find(model_id)
    except KeyError:
        return {"not_independent_on": [], "independence_unknown_on": []}
    flags = {"not_independent_on": list(entry.get("not_independent_on", [])),
             "independence_unknown_on": list(entry.get("independence_unknown_on", []))}
    if CANDIDATES.exists():
        with CANDIDATES.open("rb") as f:
            for c in tomllib.load(f).get("candidate", []):
                if c["id"] == entry.get("candidate"):
                    flags = {"not_independent_on": list(c.get("not_independent_on", [])),
                             "independence_unknown_on": list(c.get("independence_unknown_on", []))}
    return flags
