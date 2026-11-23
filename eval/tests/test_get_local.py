"""get_local.py: preparing the private local sets (synthetic audio only; nothing here touches eval/local/)."""

import json

import numpy as np
import pytest
import soundfile as sf

import get_local
from wbeval import data
from wbeval.labels import load_labels


def write_clip(folder, stem, text="नमस्ते दोस्तों", seconds=1.0, sr=48000, lang_json=None):
    folder.mkdir(parents=True, exist_ok=True)
    t = np.arange(int(seconds * sr)) / sr
    sf.write(folder / f"{stem}.wav", (0.1 * np.sin(2 * np.pi * 220 * t)).astype("float32"), sr, subtype="PCM_16")
    if text is not None:
        (folder / f"{stem}.txt").write_text(text, encoding="utf-8")
    if lang_json is not None:
        (folder / f"{stem}.lang.json").write_text(json.dumps(lang_json), encoding="utf-8")


@pytest.fixture
def dirs(tmp_path, monkeypatch):
    monkeypatch.setattr(data, "DATA_DIR", tmp_path / "data")
    return tmp_path / "local", tmp_path / "data"


def test_a_set_becomes_a_dataset_with_16k_mono_audio_and_a_manifest(dirs):
    local, out = dirs
    write_clip(local / "hindi", "c1")
    write_clip(local / "hindi", "c2", text="दूसरी क्लिप", seconds=2.0)
    report = get_local.prepare_local("hindi", local, out)
    assert report == {"set": "local-hindi", "clips": 2, "minutes": 0.05, "labelled_chunks": 0, "skipped": []}
    utts = data.load_manifest("local-hindi", out)
    assert [u.id for u in utts] == ["c1", "c2"]
    assert all(u.lang == "hi" and not u.mixed for u in utts)
    info = sf.info(out / "local-hindi" / "audio" / "c2.wav")
    assert (info.samplerate, info.channels, info.subtype) == (16000, 1, "PCM_16")
    assert 1.9 < utts[1].duration_s < 2.1
    assert "private" in (out / "local-hindi" / "source.json").read_text(encoding="utf-8")


def test_a_clip_without_a_reference_is_skipped_and_named(dirs):
    local, out = dirs
    write_clip(local / "english", "ok", text="hello there")
    write_clip(local / "english", "bare", text=None)
    write_clip(local / "english", "blank", text="   ")
    report = get_local.prepare_local("english", local, out)
    assert report["clips"] == 1 and report["skipped"] == ["bare", "blank"]
    assert data.load_manifest("local-english", out)[0].lang == "en"


def test_chunk_labels_are_written_next_to_the_set_and_never_held_out(dirs):
    local, out = dirs
    lang = {"mixed": True, "chunks": [{"start_ms": 0, "end_ms": 500, "kind": "mixed", "main": "en"},
                                      {"start_ms": 500, "end_ms": 900, "kind": "pure", "main": "hi"}]}
    write_clip(local / "codeswitch", "m1", text="मेरा main point", lang_json=lang)
    report = get_local.prepare_local("codeswitch", local, out)
    assert report["labelled_chunks"] == 2
    utt = data.load_manifest("local-codeswitch", out)[0]
    assert utt.mixed and utt.lang == "hi"
    labels = load_labels("local-codeswitch", heldout=False, data_dir=out)
    assert [(c.start_ms, c.end_ms, c.kind, c.main) for c in labels] == [(0, 500, "mixed", "en"), (500, 900, "pure", "hi")]
    assert load_labels("local-codeswitch", heldout=True, data_dir=out) == []


def test_bad_input_is_refused_with_the_file_named(dirs):
    local, out = dirs
    write_clip(local / "urdu", "u1", lang_json={"lang": "fr"})
    with pytest.raises(get_local.LocalSetError, match="u1.wav"):
        get_local.prepare_local("urdu", local, out)
    write_clip(local / "panel", "p1", lang_json={"chunks": [{"start_ms": 900, "end_ms": 100, "kind": "mixed", "main": "ur"}]})
    with pytest.raises(get_local.LocalSetError, match="p1.lang.json"):
        get_local.prepare_local("panel", local, out)
    with pytest.raises(get_local.LocalSetError, match="not found"):
        get_local.prepare_local("nothing", local, out)
    (local / "empty").mkdir()
    with pytest.raises(get_local.LocalSetError, match="no clip"):
        get_local.prepare_local("empty", local, out)


def test_the_local_folder_is_git_ignored():
    import subprocess
    from wbeval.common import REPO
    out = subprocess.run(["git", "check-ignore", "-v", "eval/local/x.wav", "eval/local/codeswitch/a.wav",
                          "eval/local/CONSENT/a.pdf"], cwd=REPO, capture_output=True, text=True)
    assert out.returncode == 0
    assert out.stdout.count("eval/local/") >= 3
