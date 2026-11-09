"""Vocabulary trim of a ggml whisper file (used for the Hinglish model's appended <|mixedcode|> token)."""

import struct
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts" / "models"))

import ggml_vocab  # noqa: E402

STATE = 4


def write_model(path: Path, n_vocab: int) -> np.ndarray:
    """A tiny file in convert-h5-to-ggml.py's layout: header, mel filters, tokens, tensors."""
    emb = np.arange(n_vocab * STATE, dtype=np.float16).reshape(n_vocab, STATE)
    with path.open("wb") as f:
        f.write(struct.pack("<i", ggml_vocab.GGML_MAGIC))
        f.write(struct.pack("<11i", n_vocab, 1500, STATE, 1, 1, 448, STATE, 1, 1, 2, 1))
        f.write(struct.pack("<ii", 2, 3))
        f.write(np.ones(6, dtype=np.float32).tobytes())
        tokens = [b"a", b"bc", b"def"]
        f.write(struct.pack("<i", len(tokens)))
        for t in tokens:
            f.write(struct.pack("<i", len(t)) + t)
        for name, arr in (("decoder.ln.weight", np.full(STATE, 7, dtype=np.float32)),
                          (ggml_vocab.TOKEN_EMBEDDING, emb),
                          ("decoder.ln.bias", np.zeros(STATE, dtype=np.float32))):
            ftype = 1 if arr.dtype == np.float16 else 0
            nb = name.encode()
            f.write(struct.pack("<iii", arr.ndim, len(nb), ftype))
            f.write(struct.pack(f"<{arr.ndim}i", *reversed(arr.shape)))
            f.write(nb)
            f.write(arr.tobytes())
    return emb


def read_embedding(path: Path) -> np.ndarray:
    data = path.read_bytes()
    name = ggml_vocab.TOKEN_EMBEDDING.encode()
    i = data.index(name)
    n_dims, name_len, _ = struct.unpack("<iii", data[i - 12 - 8:i - 8])
    cols, rows = struct.unpack("<ii", data[i - 8:i])
    start = i + len(name)
    return np.frombuffer(data[start:start + rows * cols * 2], dtype=np.float16).reshape(rows, cols)


def test_truncate_removes_last_rows_and_keeps_everything_else(tmp_path):
    src, dst = tmp_path / "in.bin", tmp_path / "out.bin"
    emb = write_model(src, 7)
    change = ggml_vocab.truncate_vocab(src, dst, 6)
    assert change == {"n_vocab_before": 7, "n_vocab_after": 6, "rows_removed": 1}
    meta = ggml_vocab.info(dst)
    assert meta["n_vocab"] == 6 and meta["token_embedding_dims"] == [STATE, 6] and meta["vocab_tokens"] == 3
    assert np.array_equal(read_embedding(dst), emb[:6])
    # Only the removed row's bytes are gone; the other tensors are copied unchanged.
    assert src.stat().st_size - dst.stat().st_size == STATE * 2
    assert dst.read_bytes().endswith(np.zeros(STATE, dtype=np.float32).tobytes())


def test_truncate_refuses_to_grow(tmp_path):
    src = tmp_path / "in.bin"
    write_model(src, 5)
    with pytest.raises(ValueError):
        ggml_vocab.truncate_vocab(src, tmp_path / "out.bin", 5)
