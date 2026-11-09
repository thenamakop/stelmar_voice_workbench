"""Locations and versions of the development tools: whisper.cpp (CLI, quantiser, converter) and
openai/whisper (mel filter assets for conversion)."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from .common import OPENAI_WHISPER_DIR, TOOLS_DIR, WHISPER_CPP_DIR

# Pinned: change only together with eval/README.md. Phase 2's wb-engine must use the same
# whisper.cpp version for the CLI comparison, or the difference is recorded.
WHISPER_CPP_TAG = "v1.9.5"
OPENAI_WHISPER_TAG = "v20250625"


def _bin(name: str) -> Path:
    for sub in ("build/bin/Release", "build/bin", "build/Release"):
        p = WHISPER_CPP_DIR / sub / f"{name}.exe"
        if p.exists():
            return p
        p = WHISPER_CPP_DIR / sub / name
        if p.exists():
            return p
    raise FileNotFoundError(f"{name} not built; run scripts/models/build_whispercpp.py")


def whisper_cli() -> Path:
    return _bin("whisper-cli")


def whisper_quantize() -> Path:
    return _bin("whisper-quantize")


def whisper_vad_tool() -> Path:
    """whisper.cpp's Silero VAD front end (examples/vad-speech-segments); used to cut chunks."""
    return _bin("whisper-vad-speech-segments")


def convert_script() -> Path:
    return WHISPER_CPP_DIR / "models" / "convert-h5-to-ggml.py"


def _git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, check=True).stdout.strip()


def whisper_cpp_info(require_binaries: bool = False) -> dict:
    """Tag, commit and built binaries of the whisper.cpp checkout; recorded in every summary."""
    recorded = TOOLS_DIR / "whisper.cpp.json"
    if not require_binaries and recorded.exists():
        return json.loads(recorded.read_text(encoding="utf-8"))
    if not WHISPER_CPP_DIR.exists():
        return {"tag": WHISPER_CPP_TAG, "commit": "not cloned", "binaries": []}
    binaries = []
    for name in ("whisper-cli", "whisper-quantize", "whisper-vad-speech-segments"):
        try:
            binaries.append(_bin(name).name)
        except FileNotFoundError:
            if require_binaries:
                raise
    return {
        "tag": WHISPER_CPP_TAG,
        "commit": _git(WHISPER_CPP_DIR, "rev-parse", "HEAD"),
        "openai_whisper_tag": OPENAI_WHISPER_TAG,
        "openai_whisper_commit": _git(OPENAI_WHISPER_DIR, "rev-parse", "HEAD") if OPENAI_WHISPER_DIR.exists() else "",
        "binaries": binaries,
    }
