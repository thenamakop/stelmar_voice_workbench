"""Cut a clip into sentence-sized chunks at VAD pauses, as the app does (docs/PLAN.md sections 5 and 6).

whisper.cpp decoding without model timestamps produces one segment per 30-second window and stops at
the token limit, skipping the rest of the window: on long clips (CoSHE-Eval, about 56 s) up to 41% of
the reference words were lost (Phase 5a review). Decoding chunks of at most `max_chunk_s` seconds, each
on its own, avoids that and matches the app, which decodes 3 to 15 s chunks cut at VAD pauses.

Speech segments come from whisper.cpp's own Silero VAD (`whisper-vad-speech-segments`, the same
model and code the engines use), so evaluation and the app cut at the same places.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

_SEGMENT = re.compile(r"Speech segment\s+\d+:\s*start\s*=\s*([0-9.]+),\s*end\s*=\s*([0-9.]+)")


@dataclass(frozen=True)
class Chunk:
    start_ms: int
    end_ms: int

    @property
    def seconds(self) -> float:
        return (self.end_ms - self.start_ms) / 1000


def parse_vad_output(text: str, duration_s: float) -> list[tuple[int, int]]:
    """Speech segments in milliseconds from whisper-vad-speech-segments' stdout.

    The tool documents its times as centiseconds; the unit is checked against the clip length so a
    change in a later whisper.cpp version cannot silently scale every chunk."""
    raw = [(float(a), float(b)) for a, b in _SEGMENT.findall(text)]
    if not raw:
        return []
    last = max(b for _, b in raw)
    if last <= duration_s * 1.05 + 0.5:
        scale = 1000.0  # seconds
    elif last <= duration_s * 105 + 50:
        scale = 10.0  # centiseconds
    else:
        raise ValueError(f"VAD segment end {last} does not fit a {duration_s:.1f} s clip in seconds or centiseconds")
    return [(round(a * scale), round(b * scale)) for a, b in raw if b > a]


def merge_segments(segments: list[tuple[int, int]], max_chunk_s: float, duration_ms: int,
                   pad_ms: int = 200) -> list[Chunk]:
    """Join consecutive speech segments into chunks no longer than max_chunk_s, cutting only in the
    pauses between segments. A single segment longer than the limit is split into equal parts (the
    VAD tool is also asked to split such segments, so this is a fallback)."""
    limit = int(max_chunk_s * 1000)
    pieces: list[tuple[int, int]] = []
    for start, end in sorted(segments):
        start, end = max(0, start - pad_ms), min(duration_ms, end + pad_ms)
        if end - start <= limit:
            pieces.append((start, end))
            continue
        n = -(-(end - start) // limit)
        step = (end - start) / n
        pieces += [(round(start + i * step), round(start + (i + 1) * step)) for i in range(n)]
    chunks: list[Chunk] = []
    cur_start = cur_end = None
    for start, end in pieces:
        if cur_start is None:
            cur_start, cur_end = start, end
            continue
        start = max(start, cur_end)  # padding may overlap the previous piece
        if end - cur_start <= limit:
            cur_end = max(cur_end, end)
        else:
            chunks.append(Chunk(cur_start, cur_end))
            cur_start, cur_end = start, end
    if cur_start is not None:
        chunks.append(Chunk(cur_start, cur_end))
    return [c for c in chunks if c.end_ms > c.start_ms]


def fixed_chunks(duration_ms: int, max_chunk_s: float) -> list[Chunk]:
    """Without VAD: equal parts of at most max_chunk_s (only for roles with vad = false)."""
    limit = int(max_chunk_s * 1000)
    n = max(1, -(-duration_ms // limit))
    step = duration_ms / n
    return [Chunk(round(i * step), round((i + 1) * step)) for i in range(n)]
