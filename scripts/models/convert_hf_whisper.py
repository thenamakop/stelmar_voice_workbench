"""Download Hugging Face Whisper fine-tunes and convert them to whisper.cpp ggml (f16).

    eval/.venv/Scripts/python scripts/models/convert_hf_whisper.py --download-only     # all candidates
    eval/.venv/Scripts/python scripts/models/convert_hf_whisper.py hindi-small hindi-medium
    eval/.venv/Scripts/python scripts/models/convert_hf_whisper.py --inspect hinglish

Wraps whisper.cpp's models/convert-h5-to-ggml.py (pinned in eval/wbeval/tools.py). Candidates and
their licences are in scripts/models/candidates.toml; anything not MIT, Apache 2.0, BSD or ISC is
dropped and recorded in models/manifest.local.json. Outputs go to models/converted/ (git-ignored):

    ggml-<id>-f16.bin      converted model (quantize_models.py makes q8_0 and q5_0 from it)
    <id>.convert.json      source, revision, file hashes, config check and token check

Before writing anything the script checks:
- mel bins: 80 for small and medium, 128 for large-v3 and its fine-tunes;
- special tokens: whisper.cpp does not read the tokenizer's special tokens; it derives their ids from
  the vocabulary size. Every id it derives is compared with the checkpoint's own added_tokens.json,
  and any extra token (such as <|mixedcode|>) is reported with what whisper.cpp will make of it.

Development time only: downloads are allowed here, never in the app. Conversion waits for the
Track A gate lock and runs at below-normal priority.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import logging
import shutil
import sys
import tempfile
import tomllib
import urllib.request
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "eval"))

from huggingface_hub import snapshot_download  # noqa: E402

from wbeval import manifest  # noqa: E402
from wbeval.common import (  # noqa: E402
    CONVERTED_DIR, MIN_FREE_GB, MIN_FREE_GB_LARGE, OPENAI_WHISPER_DIR, rel, run_heavy, setup_logging, sha256_file,
)
from wbeval.tools import convert_script, whisper_cpp_info  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ggml_vocab  # noqa: E402

log = logging.getLogger("convert")
CANDIDATES = Path(__file__).with_name("candidates.toml")
ALLOWED_LICENCES = {"mit", "apache-2.0", "bsd-2-clause", "bsd-3-clause", "isc"}
EXPECTED_MELS = {"tiny": 80, "base": 80, "small": 80, "medium": 80, "large-v2": 80, "large-v3": 128,
                 "large-v3-turbo": 128}
EXPECTED_LAYERS = {"small": 12, "medium": 24, "large-v3": 32}
SRC_DIR = CONVERTED_DIR / "_src"
DOWNLOAD_DIR = CONVERTED_DIR / "_downloads"
WEIGHT_PATTERNS = ["*.json", "*.txt", "model*.safetensors", "pytorch_model*.bin"]


def load_candidates() -> dict[str, dict]:
    with CANDIDATES.open("rb") as f:
        return {c["id"]: c for c in tomllib.load(f)["candidate"]}


def source_label(c: dict) -> str:
    return f"https://huggingface.co/{c['repo']}@{c['revision']}" if "repo" in c else c["url"]


# ------------------------------------------------------------------------------------------- download

def download_url(url: str, dest: Path) -> Path:
    """Resumable download of a public archive (development time only)."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=60) as r:
        total = int(r.headers["Content-Length"])
    part = dest.with_suffix(dest.suffix + ".part")
    if dest.exists() and dest.stat().st_size == total:
        return dest
    have = part.stat().st_size if part.exists() else 0
    req = urllib.request.Request(url, headers={"Range": f"bytes={have}-"} if have else {})
    log.info("downloading %s (%.2f GB, resuming at %.2f GB)", url, total / 1e9, have / 1e9)
    with urllib.request.urlopen(req, timeout=120) as r, part.open("ab" if have else "wb") as f:
        if have and r.status != 206:
            f.truncate(0)
        shutil.copyfileobj(r, f, length=1 << 22)
    if part.stat().st_size != total:
        raise RuntimeError(f"{url}: got {part.stat().st_size} of {total} bytes; run again to resume")
    part.replace(dest)
    return dest


def fetch(c: dict) -> tuple[Path, list[dict]]:
    """Return the checkpoint folder and the downloaded files with their SHA-256."""
    if "repo" in c:
        folder = Path(snapshot_download(c["repo"], revision=c["revision"], allow_patterns=WEIGHT_PATTERNS))
        files = [{"name": p.name, "bytes": p.stat().st_size, "sha256": sha256_file(p)}
                 for p in sorted(folder.iterdir()) if p.suffix in (".safetensors", ".bin")]
        return folder, files
    archive = download_url(c["url"], DOWNLOAD_DIR / Path(c["url"]).name)
    digest = sha256_file(archive)
    if c.get("sha256") and digest != c["sha256"]:
        raise RuntimeError(f"{archive.name}: SHA-256 {digest} does not match the pinned {c['sha256']} in "
                           "candidates.toml; the published archive changed, check it before converting")
    files = [{"name": archive.name, "bytes": archive.stat().st_size, "sha256": digest}]
    dest = SRC_DIR / c["id"]
    if not dest.exists():
        log.info("extracting %s", archive.name)
        tmp = dest.with_name(dest.name + ".tmp")
        shutil.rmtree(tmp, ignore_errors=True)
        with zipfile.ZipFile(archive) as z:
            z.extractall(tmp)
        tmp.replace(dest)
    configs = sorted(dest.rglob("config.json"))
    if c.get("model_subdir"):
        configs = [p for p in configs if p.parent.name == c["model_subdir"]]
    configs = [p for p in configs if (p.parent / "vocab.json").exists() or (p.parent / "tokenizer.json").exists()
               or any(p.parent.glob("*.safetensors")) or any(p.parent.glob("pytorch_model*.bin"))]
    if len(configs) != 1:
        raise RuntimeError(f"{c['id']}: expected one model folder in the archive, found "
                           f"{[rel(p.parent) for p in configs]}; set model_subdir in candidates.toml")
    return configs[0].parent, files


# ------------------------------------------------------------------------------------------ inspection

def whisper_cpp_special_ids(n_vocab: int) -> dict[str, int]:
    """The special-token ids whisper.cpp derives from n_vocab (src/whisper.cpp, whisper_model_load)."""
    ids = {"<|endoftext|>": 50256, "<|startoftranscript|>": 50257, "<|translate|>": 50357,
           "<|transcribe|>": 50358, "<|startoflm|>": 50359, "<|startofprev|>": 50360,
           "<|nospeech|>": 50361, "<|notimestamps|>": 50362, "<|0.00|>": 50363}
    multilingual = n_vocab >= 51865
    n_langs = n_vocab - 51765 - (1 if multilingual else 0)
    if multilingual:
        ids["<|endoftext|>"] += 1
        ids["<|startoftranscript|>"] += 1
        dt_ = n_langs - 98
        for k in list(ids)[2:]:
            ids[k] += dt_
    ids["_n_langs"] = n_langs
    return ids


def inspect(c: dict, folder: Path) -> dict:
    cfg = json.loads((folder / "config.json").read_text(encoding="utf-8"))
    vocab = json.loads((folder / "vocab.json").read_text(encoding="utf-8")) if (folder / "vocab.json").exists() else {}
    added_path = folder / "added_tokens.json"
    added = json.loads(added_path.read_text(encoding="utf-8")) if added_path.exists() else {}
    n_vocab = int(cfg["vocab_size"])
    problems: list[str] = []
    notes: list[str] = []

    mels = int(cfg["num_mel_bins"])
    if mels != EXPECTED_MELS[c["base"]]:
        problems.append(f"num_mel_bins {mels}, expected {EXPECTED_MELS[c['base']]} for {c['base']}")
    layers = int(cfg.get("encoder_layers", 0))
    if c["base"] in EXPECTED_LAYERS and layers != EXPECTED_LAYERS[c["base"]]:
        problems.append(f"encoder_layers {layers}, expected {EXPECTED_LAYERS[c['base']]} for {c['base']}")

    derived = whisper_cpp_special_ids(n_vocab)
    n_langs = derived.pop("_n_langs")
    expected_langs = 100 if c["base"].startswith("large-v3") else 99
    if n_langs != expected_langs:
        problems.append(f"vocab_size {n_vocab}: whisper.cpp will assume {n_langs} languages "
                        f"(expected {expected_langs}), shifting every task and timestamp token id")
    # Special tokens may live in added_tokens.json or (newer tokenizers) in vocab.json itself.
    known = {**vocab, **added}
    mismatched = {k: {"checkpoint": known.get(k), "whisper_cpp": v} for k, v in derived.items()
                  if k in known and known[k] != v}
    # The no-speech token was renamed between tokenizer versions.
    if "<|nospeech|>" not in known and "<|nocaptions|>" in known:
        if known["<|nocaptions|>"] != derived["<|nospeech|>"]:
            mismatched["<|nocaptions|>"] = {"checkpoint": known["<|nocaptions|>"], "whisper_cpp": derived["<|nospeech|>"]}
    if mismatched:
        problems.append(f"special-token ids differ from what whisper.cpp derives: {mismatched}")

    extra = {}
    for tok in c.get("special_tokens", []):
        tid = known.get(tok)
        if tid is None:
            extra[tok] = {"id": None, "whisper_cpp": "absent from the checkpoint's tokenizer"}
            continue
        if tid >= n_vocab:
            what = "outside vocab_size: the model has no embedding for it"
        elif tid < len(vocab) and tok in vocab:
            what = "in vocab.json, so the converter writes it as an ordinary token"
        else:
            what = "added token not written by convert-h5-to-ggml.py; whisper.cpp sees an unnamed extra id"
        extra[tok] = {"id": tid, "whisper_cpp": what}
        notes.append(f"{tok} has id {tid}: {what}")
    return {
        "n_vocab": n_vocab, "vocab_json_tokens": len(vocab), "added_tokens": len(added),
        "n_mels": mels, "encoder_layers": layers, "d_model": cfg.get("d_model"),
        "whisper_cpp_languages": n_langs, "special_token_check": "ok" if not mismatched else "mismatch",
        "extra_tokens": extra, "problems": problems, "notes": notes,
    }


# ------------------------------------------------------------------------------------------ conversion

def convert(c: dict, folder: Path) -> Path:
    out = CONVERTED_DIR / f"ggml-{c['id']}-f16.bin"
    if not (folder / "added_tokens.json").exists():
        # convert-h5-to-ggml.py opens this file but does not use it. Only write it into folders we
        # own (extracted archives), never into the Hugging Face cache.
        if folder.is_relative_to(SRC_DIR):
            (folder / "added_tokens.json").write_text("{}", encoding="utf-8")
        else:
            raise RuntimeError(f"{c['id']}: added_tokens.json missing from {folder}")
    with tempfile.TemporaryDirectory(dir=CONVERTED_DIR) as tmp:
        log_path = CONVERTED_DIR / f"{c['id']}.convert.log"
        with log_path.open("w", encoding="utf-8") as logf:
            proc = run_heavy([sys.executable, convert_script(), folder, OPENAI_WHISPER_DIR, tmp],
                             f"converting {c['id']}", stdout=logf, stderr=logf,
                             min_free_gb=MIN_FREE_GB_LARGE if c["base"].startswith("large") else MIN_FREE_GB)
        if proc.returncode != 0:
            raise RuntimeError(f"convert-h5-to-ggml.py failed for {c['id']} (exit {proc.returncode}); see {rel(log_path)}")
        Path(tmp, "ggml-model.bin").replace(out)
    return out


def process(c: dict, download_only: bool, inspect_only: bool, force: bool) -> None:
    lic = c["licence"].lower()
    if lic not in ALLOWED_LICENCES:
        manifest.drop(c["id"], f"licence {c['licence']} is not MIT, Apache 2.0, BSD or ISC", source_label(c))
        log.warning("%s dropped: licence %s", c["id"], c["licence"])
        return
    try:
        folder, files = fetch(c)
    except Exception as e:  # download gone or archive unusable: drop and record why
        manifest.drop(c["id"], f"download failed: {e}", source_label(c))
        log.exception("%s dropped: download failed", c["id"])
        return
    log.info("%s: checkpoint ready (%s)", c["id"], ", ".join(f"{f['name']} {f['bytes'] / 1e9:.2f} GB" for f in files))
    if download_only:
        return
    report = inspect(c, folder)
    for p in report["problems"]:
        log.warning("%s: %s", c["id"], p)
    for n in report["notes"]:
        log.info("%s: %s", c["id"], n)
    record = {
        "id": c["id"], "base": c["base"], "languages": c["languages"], "candidate_roles": c["candidate_roles"],
        "licence": c["licence"], "licence_evidence": c["licence_evidence"], "source": source_label(c),
        "not_independent_on": c.get("not_independent_on", []),
        "independence_unknown_on": c.get("independence_unknown_on", []),
        "training_data_evidence": c.get("training_data_evidence", ""),
        "source_files": files, "checkpoint_dir": rel(folder) if folder.is_relative_to(CONVERTED_DIR) else folder.name,
        "inspection": report, "whisper_cpp": whisper_cpp_info(), "notes": c.get("notes", ""),
    }
    rec_path = CONVERTED_DIR / f"{c['id']}.convert.json"
    if inspect_only:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return
    if any("num_mel_bins" in p or "encoder_layers" in p for p in report["problems"]):
        manifest.drop(c["id"], "; ".join(report["problems"]), source_label(c))
        log.error("%s dropped: %s", c["id"], report["problems"])
        return
    out = CONVERTED_DIR / f"ggml-{c['id']}-f16.bin"
    raw = CONVERTED_DIR / f"ggml-{c['id']}-raw-f16.bin"  # as converted, before any vocabulary trim
    target = c.get("truncate_vocab_to")
    if force or not out.exists():
        converted = convert(c, folder)
        if target:
            converted.replace(raw)
    if target:
        if force or not out.exists():
            change = ggml_vocab.truncate_vocab(raw, out, int(target))
        else:
            change = {"n_vocab_before": report["n_vocab"], "n_vocab_after": int(target),
                      "rows_removed": report["n_vocab"] - int(target)}
        derived = whisper_cpp_special_ids(int(target))
        record["vocab_truncation"] = {
            **change, "removed_tokens": [k for k, v in report["extra_tokens"].items() if v["id"] and v["id"] >= int(target)],
            "whisper_cpp_languages_after": derived["_n_langs"], "raw_file": raw.name,
            "raw_sha256": sha256_file(raw) if raw.exists() else None,
        }
        # The raw checkpoint's problems no longer apply to the trimmed file: describe the file we ship.
        record["inspection_raw"] = dict(report)
        trimmed_ids = {k: v for k, v in derived.items() if not k.startswith("_")}
        report = dict(report, n_vocab=int(target), whisper_cpp_languages=derived["_n_langs"],
                      special_token_check="ok", problems=[], notes=[
            f"vocabulary trimmed from {change['n_vocab_before']} to {target}: whisper.cpp now derives the standard "
            f"large-v3 special-token ids ({derived['_n_langs']} languages, <|transcribe|> {trimmed_ids['<|transcribe|>']}); "
            f"removed and no longer available: {record['vocab_truncation']['removed_tokens']}"])
        record["inspection"] = report
        log.info("%s: %s", c["id"], report["notes"][-1])
    record.update(file=out.name, bytes=out.stat().st_size, sha256=sha256_file(out),
                  converted_at=dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"))
    rec_path.write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    manifest.upsert({
        "id": f"{c['id']}-f16", "candidate": c["id"], "file": f"converted/{out.name}", "sha256": record["sha256"],
        "bytes": record["bytes"], "kind": "whisper", "base": c["base"], "quant": "f16", "n_mels": report["n_mels"],
        "languages": c["languages"], "candidate_roles": c["candidate_roles"], "validated_roles": [],
        "licence": c["licence"], "licence_evidence": c["licence_evidence"], "source": source_label(c),
        "not_independent_on": c.get("not_independent_on", []),
        "independence_unknown_on": c.get("independence_unknown_on", []),
        "training_data_evidence": c.get("training_data_evidence", ""),
        "whisper_cpp": f"{record['whisper_cpp']['tag']} ({record['whisper_cpp']['commit'][:12]})",
        "notes": "; ".join(filter(None, [c.get("notes", ""), *report["problems"], *report["notes"]])),
    })
    log.info("%s: wrote %s (%.2f GB, sha256 %s)", c["id"], rel(out), record["bytes"] / 1e9, record["sha256"][:16])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ids", nargs="*", help="candidate ids (default: all in candidates.toml)")
    parser.add_argument("--download-only", action="store_true", help="fetch checkpoints, do not convert")
    parser.add_argument("--inspect", action="store_true", help="print the config and token check, do not convert")
    parser.add_argument("--force", action="store_true", help="convert again even if the output exists")
    args = parser.parse_args()
    setup_logging("convert")
    CONVERTED_DIR.mkdir(parents=True, exist_ok=True)
    candidates = load_candidates()
    unknown = [i for i in args.ids if i not in candidates]
    if unknown:
        parser.error(f"unknown candidate(s) {unknown}; known: {sorted(candidates)}")
    failed = 0
    for cid in args.ids or list(candidates):
        try:
            process(candidates[cid], args.download_only, args.inspect, args.force)
        except Exception:
            log.exception("%s failed", cid)
            failed += 1
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
