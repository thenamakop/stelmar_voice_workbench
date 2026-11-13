"""Engines the harness can drive. Each transcribes one 16 kHz WAV and returns segments.

- `wb-engine`: the app's own engine, `wb-engine batch --file <wav> --models <lang=id,...>
  --langs <en,hi,ur> --out <jsonl>` (docs/prompts/phase-2-engines-and-models.md step 3). Until
  Phase 2 is merged it runs in replay mode: segments come from a recorded JSONL fixture per utterance
  (`WbEngine(fixture_dir=...)`), in the event format Phase 2 specifies.
- `whisper-cli`: whisper.cpp's own CLI from eval/.tools, the reference path for checking wb-engine
  (Phase 5a Verification: within 3 WER points). It decodes the whole clip with one model; for a
  routing setup it detects the language first (`-dl`) and then decodes with that language's model.

Both run at below-normal priority after the Track A gate lock is free.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

from . import manifest
from .audio import read_wav16, write_wav16
from .chunking import Chunk, fixed_chunks, merge_segments, parse_vad_output
from .common import MIN_FREE_GB, MIN_FREE_GB_LARGE, rel, run_heavy
from .tools import whisper_cli, whisper_cpp_info, whisper_vad_tool


@dataclass
class Segment:
    start_ms: int
    end_ms: int
    text: str
    lang: str
    lang_prob: float | None = None
    mixed: bool = False
    # wb-engine segment flags (retried, unclear, low_coverage, ...); empty for other engines.
    flags: list[str] = field(default_factory=list)


@dataclass
class Transcript:
    segments: list[Segment]
    elapsed_s: float
    model_by_segment: list[str] = field(default_factory=list)
    detected_lang: str | None = None
    detected_prob: float | None = None
    detected_outside_set: str | None = None  # detected language not among the session's languages
    chunks: list[tuple[int, int]] = field(default_factory=list)  # decoded chunks (ms), whisper-cli only

    @property
    def text(self) -> str:
        return " ".join(s.text.strip() for s in self.segments if s.text.strip())


@dataclass
class Setup:
    """One model for every language (`models = {"*": id}`) or per-language routing."""

    name: str
    models: dict[str, str]
    detector: str | None = None  # model used for language detection in routing setups

    def model_for(self, lang: str) -> str:
        return self.models.get(lang) or self.models["*"]

    @property
    def routed(self) -> bool:
        return len(set(self.models.values())) > 1


@dataclass
class Settings:
    """A role's decoding settings (eval/roles.toml)."""

    role: str
    beam_size: int = 5
    best_of: int = 5
    temperature_fallback: bool = True
    previous_text_context: bool = True
    vad: bool = True
    vad_model: str = "silero-v6.2.0"
    # Segment times come from the audio clock and VAD, never from model timestamps (docs/PLAN.md
    # section 5). Several fine-tunes were trained without timestamp tokens and hallucinate when
    # forced to emit them, so every role decodes without them.
    timestamps: bool = False
    glossary: str = ""
    threads: int = 8
    max_chunk_s: float = 30.0


class Engine:
    name = "engine"

    def describe(self) -> dict:
        raise NotImplementedError

    def transcribe(self, wav: Path, setup: Setup, langs: list[str], lang_mode: str, settings: Settings,
                   utt_id: str) -> Transcript:
        raise NotImplementedError


# ------------------------------------------------------------------------------------------- whisper-cli

_DETECTED = re.compile(r"auto-detected language:\s*(\w+)\s*\(p\s*=\s*([0-9.]+)\)")


def choose_language(detected: str | None, langs: list[str]) -> tuple[str, str | None]:
    """Decoding language for a detected one. Outside the session's languages (often hi <-> ur), never
    English (PLAN section 5 rule 3, AGENTS.md non-negotiable 6): the first non-English expected
    language. Returns (language, detected language if it was outside the set)."""
    if detected in langs:
        return detected, None
    return next((x for x in langs if x != "en"), langs[0]), detected


def min_free_gb(model: str) -> float:
    """RAM that must be free before decoding with this model (large-v3-class models need more)."""
    try:
        base = manifest.find(model).get("base", "")
    except KeyError:
        base = ""
    return MIN_FREE_GB_LARGE if base.startswith("large") else MIN_FREE_GB


class WhisperCli(Engine):
    """whisper.cpp's CLI, driven the way the app decodes: the clip is cut at VAD pauses into chunks of at
    most `max_chunk_s` seconds (wbeval.chunking, whisper.cpp's own Silero VAD) and every chunk is decoded
    on its own without model timestamps. Roles with previous-text context decode the chunks one call at
    a time and pass the last 200 characters of the previous chunk as the prompt (after the glossary);
    roles without it decode all chunks in one call (one model load per clip)."""

    name = "whisper-cli"
    # 1: whole clip per call (30 s windows; truncated long clips without timestamps).
    # 2: VAD chunks <= max_chunk_s, one decode per chunk (Phase 5a review, 9 Nov 2026).
    ADAPTER_VERSION = 2
    CONTEXT_CHARS = 200

    def describe(self) -> dict:
        return {"engine": self.name, "adapter": self.ADAPTER_VERSION, "whisper_cpp": whisper_cpp_info(),
                "binary": rel(whisper_cli())}

    def _args(self, model: str, settings: Settings) -> list[str]:
        args = [str(whisper_cli()), "-m", str(manifest.model_path(manifest.find(model))), "-t", str(settings.threads),
                "-bs", str(settings.beam_size), "-bo", str(settings.best_of), "-np"]
        if not settings.temperature_fallback:
            args.append("-nf")
        if not settings.previous_text_context:
            args += ["-mc", "0"]
        if not settings.timestamps:
            args.append("-nt")
        return args

    def chunks(self, wav: Path, settings: Settings, duration_ms: int) -> tuple[list[Chunk], float]:
        """Chunks of the clip and the seconds the VAD took."""
        if not settings.vad:
            return fixed_chunks(duration_ms, settings.max_chunk_s), 0.0
        proc = run_heavy([str(whisper_vad_tool()), "-f", str(wav),
                          "-vm", str(manifest.model_path(manifest.find(settings.vad_model))),
                          "-t", str(settings.threads), "-vmsd", f"{settings.max_chunk_s:g}"],
                         f"vad {wav.stem}", capture_output=True, text=True, encoding="utf-8", errors="replace")
        if proc.returncode != 0:
            raise RuntimeError(f"whisper-vad-speech-segments exit {proc.returncode} on {wav.name}: {proc.stderr[-500:]}")
        segments = parse_vad_output(proc.stdout, duration_ms / 1000)
        return merge_segments(segments, settings.max_chunk_s, duration_ms), proc.run_s

    def detect(self, wav: Path, model: str, settings: Settings) -> tuple[str | None, float | None]:
        proc = run_heavy([str(whisper_cli()), "-m", str(manifest.model_path(manifest.find(model))),
                          "-t", str(settings.threads), "-l", "auto", "-dl", "-f", str(wav)],
                         "whisper-cli language detection", min_free_gb=min_free_gb(model), capture_output=True,
                         text=True, encoding="utf-8", errors="replace")
        self.last_run_s = proc.run_s
        m = _DETECTED.search(proc.stderr + proc.stdout)
        return (m.group(1), float(m.group(2))) if m else (None, None)

    def _run(self, args: list[str], model: str, step: str) -> float:
        proc = run_heavy(args, step, min_free_gb=min_free_gb(model), capture_output=True, text=True,
                         encoding="utf-8", errors="replace")
        if proc.returncode != 0:
            raise RuntimeError(f"whisper-cli exit {proc.returncode} ({step}): {proc.stderr[-800:]}")
        return proc.run_s

    @staticmethod
    def _read(out: Path, chunk: Chunk, lang: str) -> tuple[list[Segment], str]:
        data = json.loads(Path(f"{out}.json").read_text(encoding="utf-8", errors="replace"))
        out_lang = data.get("result", {}).get("language", lang)
        segs = [Segment(chunk.start_ms + int(s["offsets"]["from"]), chunk.start_ms + int(s["offsets"]["to"]),
                        s["text"], out_lang) for s in data.get("transcription", [])]
        # Segment times stay inside the chunk: without model timestamps the offsets span the chunk.
        segs = [Segment(sg.start_ms, min(sg.end_ms, chunk.end_ms), sg.text, sg.lang) for sg in segs]
        return segs, out_lang

    def transcribe(self, wav, setup, langs, lang_mode, settings, utt_id):
        elapsed = 0.0  # engine time only: gate-lock, memory and busy-lock waits are left out
        detected = prob = outside = None
        if lang_mode == "fixed":
            lang = langs[0]
        else:
            detected, prob = self.detect(wav, setup.detector or setup.model_for(langs[0]), settings)
            elapsed += self.last_run_s
            # whisper-cli detects once per clip (first 30 s) among all languages; the app detects per
            # chunk among the session's languages.
            lang, outside = choose_language(detected, langs)
        model = setup.model_for(lang)
        samples = read_wav16(wav)
        duration_ms = round(len(samples) / 16)
        chunks, vad_s = self.chunks(wav, settings, duration_ms)
        elapsed += vad_s
        segments: list[Segment] = []
        out_lang = lang
        with tempfile.TemporaryDirectory() as tmp:
            files = []
            for i, c in enumerate(chunks):
                cw = Path(tmp) / f"c{i:03d}.wav"
                write_wav16(cw, samples[c.start_ms * 16:c.end_ms * 16])
                files.append((c, cw, Path(tmp) / f"c{i:03d}"))
            base = self._args(model, settings) + ["-l", lang, "-oj"]
            if settings.previous_text_context:
                previous = ""
                for i, (c, cw, out) in enumerate(files):
                    prompt = " ".join(x for x in (settings.glossary, previous[-self.CONTEXT_CHARS:]) if x).strip()
                    args = base + ["-of", str(out), "-f", str(cw)] + (["--prompt", prompt] if prompt else [])
                    elapsed += self._run(args, model, f"whisper-cli {model} {wav.stem} chunk {i + 1}/{len(files)}")
                    segs, out_lang = self._read(out, c, lang)
                    segments += segs
                    text = " ".join(sg.text.strip() for sg in segs if sg.text.strip())
                    previous = (previous + " " + text).strip() if text else previous
            elif files:
                args = base + (["--prompt", settings.glossary] if settings.glossary else [])
                for c, cw, out in files:
                    args += ["-f", str(cw), "-of", str(out)]
                elapsed += self._run(args, model, f"whisper-cli {model} {wav.stem} ({len(files)} chunks)")
                for c, cw, out in files:
                    segs, out_lang = self._read(out, c, lang)
                    segments += segs
        return Transcript(segments, elapsed, [model] * len(segments), detected or out_lang, prob, outside,
                          chunks=[(c.start_ms, c.end_ms) for c in chunks])


# --------------------------------------------------------------------------------------------- wb-engine

class WbEngine(Engine):
    """`wb-engine batch --file`, or replay of recorded events while Phase 2 is not merged."""

    name = "wb-engine"

    def __init__(self, binary: Path | None = None, fixture_dir: Path | None = None):
        self.binary = binary
        self.fixture_dir = fixture_dir
        if binary is None and fixture_dir is None:
            env = os.environ.get("WB_ENGINE")
            if not env:
                raise RuntimeError("wb-engine not available: set WB_ENGINE to the built binary, or pass a fixture "
                                   "directory (--engine-fixture) until Phase 2 is merged")
            self.binary = Path(env)

    def describe(self) -> dict:
        if self.fixture_dir is not None:
            return {"engine": self.name, "mode": "replay", "fixture": rel(self.fixture_dir)}
        version = subprocess.run([str(self.binary), "--version"], capture_output=True, text=True).stdout.strip()
        return {"engine": self.name, "mode": "process", "binary": rel(self.binary), "version": version,
                "device": os.environ.get("WB_ENGINE_DEVICE") or "cpu"}

    def command(self, wav: Path, setup: Setup, langs: list[str], settings: Settings, out: Path) -> list[str]:
        models = ",".join(f"{lang}={setup.model_for(lang)}" for lang in langs)
        cmd = [str(self.binary), "batch", "--file", str(wav), "--models", models, "--langs", ",".join(langs),
               "--out", str(out), "--settings", json.dumps(asdict(settings), separators=(",", ":"))]
        # WB_ENGINE_DEVICE=gpu:N runs the batch on a GPU of the engine build (`wb-engine info` lists the
        # indexes); unset means the engine's default (the CPU). Used by the Phase 3 informational comparison.
        device = os.environ.get("WB_ENGINE_DEVICE")
        if device:
            cmd += ["--device", device]
        return cmd

    def transcribe(self, wav, setup, langs, lang_mode, settings, utt_id):
        if lang_mode == "fixed":
            langs = langs[:1]
        t0 = time.perf_counter()
        if self.fixture_dir is not None:
            path = self.fixture_dir / f"{utt_id}.jsonl"
            if not path.exists():
                raise FileNotFoundError(f"no recorded fixture {rel(path)}")
            events = path.read_text(encoding="utf-8")
        else:
            with tempfile.TemporaryDirectory() as tmp:
                out = Path(tmp) / "segments.jsonl"
                proc = run_heavy(self.command(wav, setup, langs, settings, out), "wb-engine batch",
                                 capture_output=True, text=True, encoding="utf-8", errors="replace")
                if proc.returncode != 0:
                    raise RuntimeError(f"wb-engine exit {proc.returncode} on {wav.name}: {proc.stderr[-800:]}")
                t0 += proc.gate_wait_s  # leave the gate-lock wait out of the timing
                events = out.read_text(encoding="utf-8") if out.exists() else proc.stdout
        return parse_events(events, time.perf_counter() - t0)


class WhisperCliOnEngineChunks(WhisperCli):
    """whisper-cli decoding exactly the chunks wb-engine produces (`wb-engine chunks --greedy`, the same
    chunker, VAD model and limit as `wb-engine batch --file`), so that both sides decode identical audio and
    only the decoding is compared (Phase 2 parity gate). Previous-text context only for English: the
    context reaches whisper-cli as a --prompt argument, which Windows' ANSI command line corrupts for
    non-Latin scripts. Fails on any "??" or U+FFFD in the text, which would mean a broken character."""

    name = "whisper-cli-engine-chunks"
    ADAPTER_VERSION = 1

    def __init__(self):
        env = os.environ.get("WB_ENGINE")
        if not env:
            raise RuntimeError("whisper-cli-engine-chunks needs WB_ENGINE (the wb-engine binary that cuts the chunks)")
        self.binary = Path(env)

    def describe(self) -> dict:
        version = subprocess.run([str(self.binary), "--version"], capture_output=True, text=True).stdout.strip()
        return {**super().describe(), "chunks_from": rel(self.binary), "chunks_from_version": version}

    def chunks(self, wav: Path, settings: Settings, duration_ms: int) -> tuple[list[Chunk], float]:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "chunks.json"
            proc = run_heavy([str(self.binary), "chunks", "--file", str(wav), "--greedy",
                              "--max-chunk-s", f"{settings.max_chunk_s:g}", "--out", str(out)],
                             f"wb-engine chunks {wav.stem}", capture_output=True, text=True, encoding="utf-8",
                             errors="replace")
            if proc.returncode != 0:
                raise RuntimeError(f"wb-engine chunks exit {proc.returncode} on {wav.name}: {proc.stderr[-500:]}")
            doc = json.loads(out.read_text(encoding="utf-8"))
        return [Chunk(int(c["start_ms"]), int(c["end_ms"])) for c in doc["chunks"]], proc.run_s

    def transcribe(self, wav, setup, langs, lang_mode, settings, utt_id):
        if settings.previous_text_context and langs[0] != "en":
            raise RuntimeError("whisper-cli-engine-chunks cannot carry previous-text context for non-Latin scripts "
                               "(the --prompt argument is corrupted by Windows' ANSI command line); use role 'parity'")
        t = super().transcribe(wav, setup, langs, lang_mode, settings, utt_id)
        for s in t.segments:
            if "??" in s.text or chr(0xFFFD) in s.text:
                raise RuntimeError(f"{wav.name}: broken characters in whisper-cli's text ('??' or U+FFFD)")
        return t


def parse_events(text: str, elapsed_s: float) -> Transcript:
    """Read wb-engine JSON lines; keep `segment` events, fail on `error`."""
    segments: list[Segment] = []
    models: list[str] = []
    for line in text.splitlines():
        if not line.strip():
            continue
        ev = json.loads(line)
        kind = ev.get("type") or ev.get("event")
        if kind == "error":
            raise RuntimeError(f"wb-engine error: {ev.get('message', ev)}")
        if kind != "segment":
            continue
        segments.append(Segment(int(ev["start_ms"]), int(ev["end_ms"]), ev["text"], ev["lang"],
                                ev.get("lang_prob"), bool(ev.get("mixed", False)), list(ev.get("flags", []))))
        models.append(ev.get("model", ""))
    return Transcript(segments, elapsed_s, models)


def segments_to_events(t: Transcript, model: str) -> str:
    """Write a transcript as wb-engine events, for recording replay fixtures."""
    lines = [json.dumps({"v": 0, "type": "segment", **asdict(s), "model": model}, ensure_ascii=False)
             for s in t.segments]
    lines.append(json.dumps({"v": 0, "type": "done"}))
    return "\n".join(lines) + "\n"
