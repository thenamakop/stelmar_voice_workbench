"""Download stock whisper.cpp ggml models (zero-shot baselines and plain-Whisper fallbacks).

    eval/.venv/Scripts/python scripts/models/download_ggml.py small medium
    eval/.venv/Scripts/python scripts/models/download_ggml.py --list

Files come from the Hugging Face repository ggerganov/whisper.cpp (MIT) at a pinned revision, land in
models/ (git-ignored) and are recorded with SHA-256 in models/manifest.local.json. Development time
only: the app never downloads.
"""

from __future__ import annotations

import argparse
import logging
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "eval"))

from huggingface_hub import HfApi, hf_hub_download  # noqa: E402

from wbeval import manifest  # noqa: E402
from wbeval.common import MODELS_DIR, rel, setup_logging, sha256_file  # noqa: E402

log = logging.getLogger("download_ggml")
REPO = "ggerganov/whisper.cpp"

VAD_REPO = "ggml-org/whisper-vad"
# Repository commits read on 8 November 2026.
PINNED_REVISIONS = {
    REPO: "5359861c739e955e79d9a303bcbc70fb988958b1",
    VAD_REPO: "9ffd54a1e1ee413ddf265af9913beaf518d1639b",
}

# id -> (file, base, quant, n_mels, languages, candidate roles)
MODELS = {
    "small": ("ggml-small.bin", "small", "f16", 80, ["en", "hi", "ur"], ["near-live", "baseline"]),
    "medium": ("ggml-medium.bin", "medium", "f16", 80, ["en", "hi", "ur"], ["baseline"]),
    "large-v3-turbo-q5_0": ("ggml-large-v3-turbo-q5_0.bin", "large-v3-turbo", "q5_0", 128, ["en"], ["rolling", "live"]),
    "large-v3-q5_0": ("ggml-large-v3-q5_0.bin", "large-v3", "q5_0", 128, ["en", "hi", "ur"], ["night"]),
    "large-v3": ("ggml-large-v3.bin", "large-v3", "f16", 128, ["en", "hi", "ur"], ["live", "night", "translate"]),
    "silero-v6.2.0": ("ggml-silero-v6.2.0.bin", "silero-vad", "f32", 0, [], ["vad"]),
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ids", nargs="*")
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args()
    if args.list or not args.ids:
        for k, v in MODELS.items():
            print(f"{k:22} {v[0]}")
        return 0
    setup_logging("download_ggml")
    api = HfApi()
    revisions: dict[str, str] = {}
    for mid in args.ids:
        if mid not in MODELS:
            parser.error(f"unknown model {mid}")
        fname, base, quant, mels, langs, roles = MODELS[mid]
        is_vad = base == "silero-vad"
        repo = VAD_REPO if is_vad else REPO
        revision = PINNED_REVISIONS.get(repo) or revisions.setdefault(repo, api.model_info(repo).sha)
        dest = MODELS_DIR / fname
        if not dest.exists():
            log.info("downloading %s from %s@%s", fname, repo, revision[:12])
            cached = Path(hf_hub_download(repo, fname, revision=revision))
            shutil.copyfile(cached, dest)
        entry = {
            "id": mid, "candidate": mid, "file": fname, "sha256": sha256_file(dest), "bytes": dest.stat().st_size,
            "kind": "vad" if is_vad else "whisper", "base": base, "quant": quant, "n_mels": mels, "languages": langs,
            "candidate_roles": roles, "validated_roles": ["vad"] if is_vad else [], "licence": "MIT",
            "licence_evidence": "snakers4/silero-vad LICENSE (MIT)" if is_vad
            else "openai/whisper LICENSE (MIT); ggerganov/whisper.cpp model card",
            "source": f"https://huggingface.co/{repo}@{revision}",
            "notes": "Silero VAD converted by whisper.cpp" if is_vad else "stock OpenAI weights, converted by whisper.cpp",
        }
        manifest.upsert(entry)
        log.info("%s: %s %.2f GB sha256 %s", mid, rel(dest), entry["bytes"] / 1e9, entry["sha256"][:16])
    return 0


if __name__ == "__main__":
    sys.exit(main())
