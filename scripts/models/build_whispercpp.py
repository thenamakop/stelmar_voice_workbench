"""Clone (if needed) and build whisper.cpp's whisper-cli, whisper-quantize and whisper-vad-speech-segments
(Silero VAD, used to cut chunks) for evaluation and conversion.

    eval/.venv/Scripts/python scripts/models/build_whispercpp.py [--jobs 8]

Development time only. The build is CPU-only and portable (GGML_NATIVE=OFF with the avx2 engine's
flags), like the release engines, so evaluation numbers never depend on build-machine-only
instructions. It waits for the Track A gate lock and runs at below-normal priority.

The whisper.cpp tag and commit are written to eval/.tools/whisper.cpp.json; eval/README.md and every
results summary record them. Phase 2's wb-engine bundles whisper.cpp through whisper-rs-sys: the
comparison between wb-engine and whisper-cli must use the same whisper.cpp version, or record the
difference.
"""

from __future__ import annotations

import argparse
import json
import logging
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "eval"))

from wbeval.common import OPENAI_WHISPER_DIR, TOOLS_DIR, WHISPER_CPP_DIR, run_heavy, setup_logging  # noqa: E402
from wbeval.tools import WHISPER_CPP_TAG, OPENAI_WHISPER_TAG, whisper_cpp_info  # noqa: E402

log = logging.getLogger("build_whispercpp")

# The avx2 engine's portable flag set (docs/PLAN.md section 11).
CMAKE_FLAGS = [
    "-DGGML_NATIVE=OFF", "-DGGML_AVX=ON", "-DGGML_AVX2=ON", "-DGGML_FMA=ON", "-DGGML_F16C=ON",
    "-DGGML_AVX512=OFF", "-DGGML_CUDA=OFF", "-DGGML_VULKAN=OFF", "-DBUILD_SHARED_LIBS=OFF",
    "-DWHISPER_BUILD_TESTS=OFF", "-DWHISPER_BUILD_SERVER=OFF", "-DWHISPER_BUILD_EXAMPLES=ON",
]


def clone(url: str, tag: str, dest: Path) -> None:
    if dest.exists():
        return
    log.info("cloning %s %s", url, tag)
    subprocess.run(["git", "-c", "advice.detachedHead=false", "clone", "--depth", "1", "--branch", tag, url,
                    str(dest)], check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--jobs", type=int, default=8, help="parallel compile jobs (default 8)")
    args = parser.parse_args()
    setup_logging("build_whispercpp")
    TOOLS_DIR.mkdir(parents=True, exist_ok=True)
    clone("https://github.com/ggml-org/whisper.cpp", WHISPER_CPP_TAG, WHISPER_CPP_DIR)
    clone("https://github.com/openai/whisper", OPENAI_WHISPER_TAG, OPENAI_WHISPER_DIR)

    build = WHISPER_CPP_DIR / "build"
    run_heavy(["cmake", "-S", WHISPER_CPP_DIR, "-B", build, *CMAKE_FLAGS], "whisper.cpp configure", check=True)
    run_heavy(["cmake", "--build", build, "--config", "Release", "--parallel", str(args.jobs),
               "--target", "whisper-cli", "whisper-quantize", "whisper-vad-speech-segments"],
               "whisper.cpp build", check=True)
    info = whisper_cpp_info(require_binaries=True)
    info["cmake_flags"] = CMAKE_FLAGS
    (TOOLS_DIR / "whisper.cpp.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    log.info("built whisper.cpp %s (%s): %s", info["tag"], info["commit"][:12], ", ".join(info["binaries"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
