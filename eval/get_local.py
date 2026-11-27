"""Prepare the local STELMAR test sets (eval/local/) for the harness. Nothing here is committed.

    eval/.venv/Scripts/python eval/get_local.py                # every set found in eval/local/
    eval/.venv/Scripts/python eval/get_local.py --set hindi    # one set

Input layout (private; `eval/local/` is git-ignored, see docs/TEST-AUDIO.md and the Phase 5 plan):

    eval/local/<set>/set.json         {"lang": "hi", "mixed": true, "purpose": "..."}   set defaults
    eval/local/<set>/<clip>.wav       the recording as made (any rate; 16 kHz mono is written below)
    eval/local/<set>/<clip>.txt       the reference transcript, corrected by ear (UTF-8)
    eval/local/<set>/<clip>.lang.json optional: {"lang": "hi", "mixed": true, "chunks": [
                                          {"start_ms": 0, "end_ms": 4200, "kind": "mixed", "main": "en"}]}

Output (git-ignored): eval/data/local-<set>/ with manifest.jsonl, audio/<clip>.wav (16 kHz mono PCM16), source.json
and, when any clip has chunk labels, labels.json (the format of eval/labels/<dataset>.json, never held out).
A clip with no .txt is skipped and reported: an answer key with holes would flatter nobody and mislead everyone.
"""

from __future__ import annotations

import argparse
import json
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from wbeval import audio, data  # noqa: E402
from wbeval.common import EVAL_DIR, sha256_file, setup_logging  # noqa: E402

log = logging.getLogger("get_local")
LOCAL_DIR = EVAL_DIR / "local"
LANGS = {"en", "hi", "ur"}
DEFAULTS = {
    "english": {"lang": "en", "mixed": False},
    "hindi": {"lang": "hi", "mixed": False},
    "urdu": {"lang": "ur", "mixed": False},
    "codeswitch": {"lang": "hi", "mixed": True},
    "panel": {"lang": "ur", "mixed": True},
    "live": {"lang": "hi", "mixed": True},
}


class LocalSetError(RuntimeError):
    pass


def _json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        raise LocalSetError(f"{path}: {e}") from e


def prepare_local(set_name: str, local_dir: Path | None = None, data_dir: Path | None = None) -> dict:
    """Prepare eval/local/<set_name>/ as the dataset `local-<set_name>`. Returns a small report (counts only)."""
    src_dir = (local_dir or LOCAL_DIR) / set_name
    if not src_dir.is_dir():
        raise LocalSetError(f"{src_dir} not found")
    name = f"local-{set_name}"
    root = data_dir or data.DATA_DIR
    out_dir = root / name
    defaults = {**DEFAULTS.get(set_name, {"lang": "hi", "mixed": False})}
    set_json = src_dir / "set.json"
    if set_json.exists():
        defaults.update(_json(set_json))
    utterances: list[data.Utterance] = []
    chunks: list[dict] = []
    skipped: list[str] = []
    for wav in sorted(src_dir.glob("*.wav")):
        txt = wav.with_suffix(".txt")
        if not txt.exists() or not txt.read_text(encoding="utf-8").strip():
            skipped.append(wav.stem)
            continue
        meta = {**defaults}
        lang_json = wav.with_suffix(".lang.json")
        clip_labels = []
        if lang_json.exists():
            raw = _json(lang_json)
            meta.update({k: raw[k] for k in ("lang", "mixed") if k in raw})
            clip_labels = raw.get("chunks", [])
        if meta["lang"] not in LANGS:
            raise LocalSetError(f"{wav.name}: lang {meta['lang']!r} is not one of {sorted(LANGS)}")
        rel_audio = f"audio/{wav.stem}.wav"
        duration = audio.convert_bytes(wav.read_bytes(), out_dir / rel_audio)
        utterances.append(data.Utterance(
            id=wav.stem, audio=rel_audio, text=txt.read_text(encoding="utf-8").strip(), lang=meta["lang"],
            mixed=bool(meta["mixed"]), duration_s=round(duration, 3), sha256=sha256_file(out_dir / rel_audio)))
        for c in clip_labels:
            if c["kind"] not in ("pure", "mixed") or c["main"] not in LANGS or c["end_ms"] <= c["start_ms"]:
                raise LocalSetError(f"{lang_json.name}: bad chunk label {c}")
            chunks.append({"id": wav.stem, "start_ms": int(c["start_ms"]), "end_ms": int(c["end_ms"]),
                           "kind": c["kind"], "main": c["main"]})
    if not utterances:
        raise LocalSetError(f"{src_dir}: no clip has both a .wav and a non-empty .txt")
    src = data.Source(
        set=name, title=f"STELMAR local set: {set_name}", url="(private recordings, not published)", revision="local",
        licence="private: STELMAR recordings, speaker consent on file; never committed or published",
        licence_evidence="eval/local/CONSENT (private)", terms="evaluation only, on the developer's computers",
        notes=defaults.get("purpose", ""))
    previous = data.DATA_DIR
    data.DATA_DIR = root  # write_source and set_dir read this at call time (tests point it at a temporary folder)
    try:
        data.write_source(src, utterances)
    finally:
        data.DATA_DIR = previous
    if chunks:
        (out_dir / "labels.json").write_text(json.dumps({
            "dataset": name, "heldout": False,
            "about": "Hand labels of chunk languages from the clips' .lang.json files (private, not committed).",
            "chunks": chunks}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    log.info("%s: %d clips, %.1f min, %d labelled chunks, %d skipped (no reference)", name, len(utterances),
             sum(u.duration_s for u in utterances) / 60, len(chunks), len(skipped))
    return {"set": name, "clips": len(utterances), "minutes": round(sum(u.duration_s for u in utterances) / 60, 2),
            "labelled_chunks": len(chunks), "skipped": skipped}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--set", help="one set folder name under eval/local/ (default: all)")
    parser.add_argument("--local-dir", type=Path, default=LOCAL_DIR)
    args = parser.parse_args()
    setup_logging("get_local")
    if not args.local_dir.is_dir():
        log.error("%s does not exist: the local sets are not recorded yet (docs/TEST-AUDIO.md)", args.local_dir)
        return 2
    names = [args.set] if args.set else sorted(p.name for p in args.local_dir.iterdir()
                                              if p.is_dir() and p.name != "CONSENT")
    failed = 0
    for n in names:
        try:
            prepare_local(n, args.local_dir)
        except LocalSetError as e:
            log.error("%s", e)
            failed += 1
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
