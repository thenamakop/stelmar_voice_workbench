"""Read and trim the vocabulary of a whisper.cpp ggml model file (the format convert-h5-to-ggml.py writes).

whisper.cpp does not store special-token ids; it derives them from n_vocab (src/whisper.cpp,
whisper_model_load). A fine-tune that appends a token after the standard vocabulary, such as
Trelis/whisper-hinglish-preview's <|mixedcode|> (id 51866, n_vocab 51867), makes whisper.cpp
assume one more language and shifts every task and timestamp token. `truncate_vocab` writes a copy
with n_vocab set to the standard size and the extra rows of the (tied) token embedding removed,
so the standard ids line up again. The extra token can then no longer be produced or prompted.

    eval/.venv/Scripts/python scripts/models/ggml_vocab.py info <model.bin>
    eval/.venv/Scripts/python scripts/models/ggml_vocab.py truncate <in.bin> <out.bin> 51866
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path
from typing import BinaryIO

GGML_MAGIC = 0x67676D6C
N_HPARAMS = 11  # n_vocab .. ftype
TOKEN_EMBEDDING = "decoder.token_embedding.weight"
BYTES_PER = {0: 4, 1: 2}  # ftype 0 = f32, 1 = f16 (only unquantised files are handled)
CHUNK = 1 << 24


def _i32(f: BinaryIO) -> int:
    data = f.read(4)
    if len(data) != 4:
        raise EOFError("unexpected end of ggml file")
    return struct.unpack("<i", data)[0]


def _copy(src: BinaryIO, dst: BinaryIO, n: int) -> None:
    while n > 0:
        block = src.read(min(n, CHUNK))
        if not block:
            raise EOFError("unexpected end of ggml file")
        dst.write(block)
        n -= len(block)


def _skip(src: BinaryIO, n: int) -> None:
    src.seek(n, 1)


def read_header(f: BinaryIO) -> dict:
    if _i32(f) != GGML_MAGIC:
        raise ValueError("not a ggml whisper model")
    names = ["n_vocab", "n_audio_ctx", "n_audio_state", "n_audio_head", "n_audio_layer", "n_text_ctx",
             "n_text_state", "n_text_head", "n_text_layer", "n_mels", "ftype"]
    return dict(zip(names, (_i32(f) for _ in range(N_HPARAMS))))


def info(path: Path) -> dict:
    with path.open("rb") as f:
        h = read_header(f)
        n_mel, n_fft = _i32(f), _i32(f)
        _skip(f, 4 * n_mel * n_fft)
        h["vocab_tokens"] = _i32(f)
        for _ in range(h["vocab_tokens"]):
            _skip(f, _i32(f))
        while True:
            head = f.read(12)
            if not head:
                break
            n_dims, name_len, ftype = struct.unpack("<iii", head)
            dims = [_i32(f) for _ in range(n_dims)]
            name = f.read(name_len).decode()
            n = 1
            for d in dims:
                n *= d
            if name == TOKEN_EMBEDDING:
                h["token_embedding_dims"] = dims
            _skip(f, n * BYTES_PER[ftype])
    return h


def truncate_vocab(src_path: Path, dst_path: Path, n_vocab: int) -> dict:
    """Copy src to dst with n_vocab rows; returns what changed."""
    with src_path.open("rb") as src, dst_path.open("wb") as dst:
        h = read_header(src)
        old = h["n_vocab"]
        if n_vocab >= old:
            raise ValueError(f"n_vocab {n_vocab} is not smaller than the file's {old}")
        if h["ftype"] not in BYTES_PER:
            raise ValueError("truncate the f16 or f32 file before quantising")
        dst.write(struct.pack("<i", GGML_MAGIC))
        h_out = dict(h, n_vocab=n_vocab)
        dst.write(struct.pack(f"<{N_HPARAMS}i", *h_out.values()))
        n_mel, n_fft = _i32(src), _i32(src)
        dst.write(struct.pack("<ii", n_mel, n_fft))
        _copy(src, dst, 4 * n_mel * n_fft)
        n_tokens = _i32(src)
        if n_tokens > n_vocab:
            raise ValueError(f"the file names {n_tokens} tokens, more than {n_vocab}")
        dst.write(struct.pack("<i", n_tokens))
        for _ in range(n_tokens):
            length = _i32(src)
            dst.write(struct.pack("<i", length))
            _copy(src, dst, length)
        trimmed = False
        while True:
            head = src.read(12)
            if not head:
                break
            n_dims, name_len, ftype = struct.unpack("<iii", head)
            dims = [_i32(src) for _ in range(n_dims)]
            name_b = src.read(name_len)
            n = 1
            for d in dims:
                n *= d
            size = n * BYTES_PER[ftype]
            if name_b.decode() == TOKEN_EMBEDDING:
                # Stored as dims [n_text_state, n_vocab]: rows of n_text_state values, one per token.
                if dims[1] != old:
                    raise ValueError(f"token embedding has {dims[1]} rows, header says {old}")
                row = dims[0] * BYTES_PER[ftype]
                dst.write(struct.pack("<iii", n_dims, name_len, ftype))
                dst.write(struct.pack(f"<{n_dims}i", dims[0], n_vocab))
                dst.write(name_b)
                _copy(src, dst, row * n_vocab)
                _skip(src, row * (old - n_vocab))
                trimmed = True
                continue
            dst.write(head)
            dst.write(struct.pack(f"<{n_dims}i", *dims))
            dst.write(name_b)
            _copy(src, dst, size)
        if not trimmed:
            raise ValueError(f"{TOKEN_EMBEDDING} not found")
    return {"n_vocab_before": old, "n_vocab_after": n_vocab, "rows_removed": old - n_vocab}


def main() -> int:
    if len(sys.argv) >= 3 and sys.argv[1] == "info":
        print(info(Path(sys.argv[2])))
        return 0
    if len(sys.argv) == 5 and sys.argv[1] == "truncate":
        print(truncate_vocab(Path(sys.argv[2]), Path(sys.argv[3]), int(sys.argv[4])))
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
