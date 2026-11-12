"""Decode any input clip and write 16 kHz mono 16-bit PCM WAV, the format every evaluation set uses."""

from __future__ import annotations

import io
import shutil
import subprocess
from pathlib import Path

import numpy as np
import soundfile as sf
import soxr

TARGET_SR = 16_000


def _decode_with_ffmpeg(data: bytes) -> tuple[np.ndarray, int]:
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise RuntimeError("clip format not readable by libsndfile and ffmpeg is not on PATH")
    out = subprocess.run(
        [ffmpeg, "-hide_banner", "-loglevel", "error", "-i", "pipe:0", "-f", "f32le", "-ac", "1",
         "-ar", str(TARGET_SR), "pipe:1"],
        input=data, capture_output=True, check=True,
    )
    return np.frombuffer(out.stdout, dtype=np.float32), TARGET_SR


def decode(data: bytes) -> tuple[np.ndarray, int]:
    """Decode encoded audio bytes to float32 samples (mono mix-down) and their rate."""
    try:
        samples, sr = sf.read(io.BytesIO(data), dtype="float32", always_2d=True)
    except (sf.LibsndfileError, RuntimeError):
        return _decode_with_ffmpeg(data)
    return samples.mean(axis=1), sr


def to_16k_mono(samples: np.ndarray, sr: int) -> np.ndarray:
    if samples.ndim > 1:
        samples = samples.mean(axis=1)
    if sr != TARGET_SR:
        samples = soxr.resample(samples, sr, TARGET_SR, quality="HQ")
    return np.clip(samples, -1.0, 1.0).astype(np.float32)


def write_wav16(path: Path, samples: np.ndarray) -> float:
    """Write 16 kHz mono PCM16 and return the duration in seconds."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".wav.tmp")  # written whole, then renamed: a re-run never reuses a partial file
    sf.write(tmp, samples, TARGET_SR, subtype="PCM_16", format="WAV")
    tmp.replace(path)
    return len(samples) / TARGET_SR


def convert_file(src: Path, path: Path) -> float:
    """Decode a file on disk (m4a/mp4 needs a seekable input: its index is at the end) to 16 kHz mono PCM16 WAV.

    ffmpeg writes a mono WAV at the file's own rate and soxr does the resampling, as for every other set. The
    ffmpeg on this machine's PATH is a 2013 build: only options it knows are used.
    """
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise RuntimeError("ffmpeg is not on PATH")
    tmp = path.with_suffix(".decode.wav")
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        subprocess.run([ffmpeg, "-loglevel", "error", "-y", "-i", str(src), "-vn", "-ac", "1", "-f", "wav", str(tmp)],
                       capture_output=True, check=True)
        samples, sr = sf.read(tmp, dtype="float32", always_2d=True)
    finally:
        tmp.unlink(missing_ok=True)
    return write_wav16(path, to_16k_mono(samples.mean(axis=1), sr))


def convert_bytes(data: bytes, path: Path) -> float:
    samples, sr = decode(data)
    return write_wav16(path, to_16k_mono(samples, sr))


def read_wav16(path: Path) -> np.ndarray:
    samples, sr = sf.read(path, dtype="float32", always_2d=True)
    return to_16k_mono(samples.mean(axis=1), sr)
