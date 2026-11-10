"""Harness end to end with the wb-engine replay stub, engine event parsing, tables, data provenance
and the whisper.cpp special-token check used by the converter."""

import importlib.util
import json
import sys
from pathlib import Path

import numpy as np
import pytest

import run as harness
from wbeval import data
from wbeval.audio import write_wav16
from wbeval.engines import Segment, Setup, Transcript, WbEngine, parse_events, segments_to_events

REPO = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def make_set(tmp: Path, name: str, rows: list[tuple[str, str, str, bool]]) -> Path:
    """A tiny dataset with silent WAVs; replay never reads the audio but the manifest points at it."""
    data_dir = tmp / "data"
    utts = []
    for uid, text, lang, mixed in rows:
        wav = data_dir / name / "audio" / f"{uid}.wav"
        dur = write_wav16(wav, np.zeros(16000, dtype=np.float32))
        utts.append(data.Utterance(uid, f"audio/{uid}.wav", text, lang, mixed, dur, "0" * 64))
    (data_dir / name).mkdir(parents=True, exist_ok=True)
    (data_dir / name / "manifest.jsonl").write_text(
        "".join(json.dumps(u.__dict__, ensure_ascii=False) + "\n" for u in utts), encoding="utf-8")
    return data_dir


def write_fixture(folder: Path, uid: str, segments: list[Segment]) -> None:
    folder.mkdir(parents=True, exist_ok=True)
    (folder / f"{uid}.jsonl").write_text(segments_to_events(Transcript(segments, 0.0), "medium"), encoding="utf-8")


def test_replay_run_scores_and_writes_tables(tmp_path):
    data_dir = make_set(tmp_path, "mini", [
        ("a", "मैं office जा रहा हूँ।", "hi", True),
        ("b", "आज मौसम अच्छा है।", "hi", False),
    ])
    fx = tmp_path / "fixture"
    write_fixture(fx, "a", [Segment(0, 900, "मैं ऑफिस जा रहा हूँ", "hi", 0.97)])
    write_fixture(fx, "b", [Segment(0, 500, "आज मौसम", "hi", 0.9), Segment(500, 1000, "अच्छा है", "en", 0.6)])
    results = tmp_path / "results"
    s = harness.run("wb-engine", Setup("medium", {"*": "medium"}), "mini", "rolling", fixture=str(fx),
                    data_dir=data_dir, results_dir=results, machine="test", lang_mode="detect", langs=["en", "hi"])
    assert s["utterances"] == 2 and s["failures"] == 0
    assert s["wer"] == pytest.approx(1 / 9)  # office -> ऑफिस, 9 reference words
    assert s["cer"] is not None
    assert s["latin_words"]["devanagari_rate"] == 1.0
    assert s["lang_detection"]["chunks"] == 3
    assert s["lang_detection"]["accuracy"] == pytest.approx(2 / 3)
    assert s["lang_detection"]["confusion"] == {"hi->en": 1, "hi->hi": 2}
    assert s["engine"]["mode"] == "replay"
    assert s["whisper_cpp"]["tag"]  # recorded in every summary
    assert s["settings"]["beam_size"] == 5

    runs_file = tmp_path / "runs.toml"
    runs_file.write_text(f'[[compare]]\na = "{s["run"]}"\nb = "{s["run"]}"\nmax_wer_points = 3.0\n', encoding="utf-8")
    tables = harness.write_tables(results, runs_file).read_text(encoding="utf-8")
    assert s["run"] in tables and "| PASS |" in tables


def test_replay_run_resumes_and_records_failures(tmp_path):
    data_dir = make_set(tmp_path, "mini", [("a", "one two", "en", False), ("b", "three", "en", False)])
    fx = tmp_path / "fixture"
    write_fixture(fx, "a", [Segment(0, 1000, "one two", "en")])
    results = tmp_path / "results"
    s = harness.run("wb-engine", Setup("m", {"*": "m"}), "mini", "near-live", fixture=str(fx), data_dir=data_dir,
                    results_dir=results)
    assert s["failures"] == 1 and s["utterances"] == 1
    write_fixture(fx, "b", [Segment(0, 1000, "three", "en")])
    s = harness.run("wb-engine", Setup("m", {"*": "m"}), "mini", "near-live", fixture=str(fx), data_dir=data_dir,
                    results_dir=results)
    assert s["failures"] == 0 and s["utterances"] == 2 and s["wer"] == 0.0
    assert s["settings"]["beam_size"] == 1  # near-live is greedy


def test_committed_fixture_replays():
    """The hand-written fixture parses as wb-engine events (no dataset text is committed)."""
    folder = FIXTURES / "wb-engine" / "synthetic-hi"
    files = sorted(folder.glob("*.jsonl"))
    assert files, "fixture missing"
    for f in files:
        t = parse_events(f.read_text(encoding="utf-8"), 0.0)
        assert t.segments and all(s.lang == "hi" for s in t.segments)
        letters = [c for c in t.text if c.isalpha()]
        assert sum("ऀ" <= c <= "ॿ" for c in letters) / len(letters) > 0.9


def test_parse_events_fails_on_engine_error():
    with pytest.raises(RuntimeError):
        parse_events('{"type":"error","message":"model missing"}\n', 0.0)


def test_parse_events_skips_other_events():
    text = '{"type":"ready"}\n{"type":"segment","start_ms":0,"end_ms":10,"text":"x","lang":"en"}\n{"type":"done"}\n'
    assert len(parse_events(text, 0.0).segments) == 1


def test_wb_engine_command_matches_phase_2_spec(tmp_path):
    eng = WbEngine(binary=Path("wb-engine.exe"))
    cmd = eng.command(Path("clip.wav"), Setup("r", {"en": "medium", "hi": "hindi-medium-q8_0"}), ["en", "hi"],
                      harness.role_settings("rolling"), tmp_path / "o.jsonl")
    assert cmd[1:4] == ["batch", "--file", "clip.wav"]
    assert cmd[cmd.index("--models") + 1] == "en=medium,hi=hindi-medium-q8_0"
    assert cmd[cmd.index("--langs") + 1] == "en,hi"
    assert "--out" in cmd


def test_wb_engine_without_binary_or_fixture_is_an_error(monkeypatch):
    monkeypatch.delenv("WB_ENGINE", raising=False)
    with pytest.raises(RuntimeError, match="Phase 2"):
        WbEngine()


def test_setup_parsing():
    assert harness.parse_setup("medium", {}).model_for("hi") == "medium"
    s = harness.parse_setup("en=medium,hi=indic", {})
    assert s.model_for("en") == "medium" and s.model_for("hi") == "indic" and s.routed


def test_select_is_deterministic_and_spread():
    utts = [data.Utterance(f"{i:03d}", "", "", "en", False, 1.0, "") for i in range(100)]
    picked = harness.select(utts, 20)
    assert len(picked) == 20 and picked[0].id == "000" and picked[-1].id == "095"
    assert picked == harness.select(list(reversed(utts)), 20)


def test_sources_md_lists_licence_and_checksum(tmp_path, monkeypatch):
    monkeypatch.setattr(data, "DATA_DIR", tmp_path)
    src = data.Source(set="mini", title="Mini", url="https://example.org/mini", revision="abc123", licence="cc-by-4.0",
                      licence_evidence="card", terms="evaluation",
                      files=[data.SourceFile("a.tar.gz", "https://example.org/a.tar.gz", 10, "f" * 64)])
    data.write_source(src, [data.Utterance("x", "audio/x.wav", "t", "en", False, 3600.0, "0" * 64)])
    md = (tmp_path / "SOURCES.md").read_text(encoding="utf-8")
    assert "cc-by-4.0" in md and "f" * 64 in md and "| mini | 1 | 1.00 |" in md


def _load_converter():
    path = REPO / "scripts" / "models" / "convert_hf_whisper.py"
    spec = importlib.util.spec_from_file_location("convert_hf_whisper", path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["convert_hf_whisper"] = mod
    spec.loader.exec_module(mod)
    return mod


def test_whisper_cpp_special_token_ids():
    conv = _load_converter()
    v2 = conv.whisper_cpp_special_ids(51865)  # small, medium, large-v2: 99 languages
    assert v2["_n_langs"] == 99 and v2["<|startoftranscript|>"] == 50258 and v2["<|transcribe|>"] == 50359
    assert v2["<|notimestamps|>"] == 50363
    v3 = conv.whisper_cpp_special_ids(51866)  # large-v3: 100 languages
    assert v3["_n_langs"] == 100 and v3["<|transcribe|>"] == 50360 and v3["<|notimestamps|>"] == 50364
    extra = conv.whisper_cpp_special_ids(51867)  # one added token shifts every task id
    assert extra["_n_langs"] == 101 and extra["<|transcribe|>"] == 50361


def test_converter_flags_an_added_token(tmp_path):
    conv = _load_converter()
    vocab = {f"t{i}": i for i in range(50257)}
    added = {"<|endoftext|>": 50257, "<|startoftranscript|>": 50258, "<|transcribe|>": 50360,
             "<|notimestamps|>": 50364, "<|mixedcode|>": 51866}
    (tmp_path / "vocab.json").write_text(json.dumps(vocab), encoding="utf-8")
    (tmp_path / "added_tokens.json").write_text(json.dumps(added), encoding="utf-8")
    (tmp_path / "config.json").write_text(json.dumps({"vocab_size": 51867, "num_mel_bins": 128, "encoder_layers": 32}),
                                          encoding="utf-8")
    report = conv.inspect({"id": "x", "base": "large-v3", "special_tokens": ["<|mixedcode|>"]}, tmp_path)
    assert report["whisper_cpp_languages"] == 101
    assert any("shifting every task" in p for p in report["problems"])
    assert report["special_token_check"] == "mismatch"
    assert report["extra_tokens"]["<|mixedcode|>"]["id"] == 51866


@pytest.mark.parametrize("role", ["near-live", "rolling", "night", "live"])
def test_every_role_decodes_without_model_timestamps(role):
    # PLAN section 5: times come from the audio clock and VAD. vasista22 fine-tunes hallucinate when
    # decoded with timestamp tokens (hindi-small: 67.8% WER on FLEURS hi with them).
    assert harness.role_settings(role).timestamps is False


def test_fixed_mode_does_not_report_detection_accuracy(tmp_path):
    data_dir = make_set(tmp_path, "mini", [("a", "नमस्ते", "hi", False)])
    fx = tmp_path / "fixture"
    write_fixture(fx, "a", [Segment(0, 1000, "नमस्ते", "hi")])
    s = harness.run("wb-engine", Setup("m", {"*": "m"}), "mini", "rolling", fixture=str(fx), data_dir=data_dir,
                    results_dir=tmp_path / "r")
    assert s["lang_detection"]["accuracy"] is None and s["language_source"].startswith("given")


def test_changed_settings_refuse_to_resume(tmp_path):
    data_dir = make_set(tmp_path, "mini", [("a", "one", "en", False)])
    fx = tmp_path / "fixture"
    write_fixture(fx, "a", [Segment(0, 1000, "one", "en")])
    kw = dict(fixture=str(fx), data_dir=data_dir, results_dir=tmp_path / "r", name="same")
    harness.run("wb-engine", Setup("m", {"*": "m"}), "mini", "rolling", **kw)
    with pytest.raises(SystemExit, match="fingerprint"):
        harness.run("wb-engine", Setup("m", {"*": "m"}), "mini", "near-live", **kw)
    s = harness.run("wb-engine", Setup("m", {"*": "m"}), "mini", "near-live", force=True, **kw)
    assert s["settings"]["role"] == "near-live"


def test_trained_on_set_is_marked_not_independent(tmp_path, monkeypatch):
    data_dir = make_set(tmp_path, "fleurs-hi", [("a", "नमस्ते", "hi", False)])
    fx = tmp_path / "fixture"
    write_fixture(fx, "a", [Segment(0, 1000, "नमस्ते", "hi")])
    entries = {"hindi-small-q8_0": {"file": "x", "sha256": "s", "not_independent_on": ["fleurs-hi"]},
               "silero-v6.2.0": {"file": "v", "sha256": "v"}}
    monkeypatch.setattr(harness.manifest, "find", lambda mid: entries[mid])
    s = harness.run("wb-engine", Setup("hindi-small-q8_0", {"*": "hindi-small-q8_0"}), "fleurs-hi", "near-live",
                    fixture=str(fx), data_dir=data_dir, results_dir=tmp_path / "r")
    assert s["independent"] is False and s["not_independent_models"] == ["hindi-small-q8_0"]
    assert "**no** (hindi-small-q8_0)" in harness.write_tables(tmp_path / "r", tmp_path / "none.toml").read_text(
        encoding="utf-8")


def test_unknown_role_setting_is_an_error(tmp_path, monkeypatch):
    roles = tmp_path / "roles.toml"
    roles.write_text("[rolling]\nbeam_sise = 5\n", encoding="utf-8")
    monkeypatch.setattr(harness, "ROLES_FILE", roles)
    with pytest.raises(SystemExit, match="beam_sise"):
        harness.role_settings("rolling")


def test_nukta_ignored_score_and_rank_flip_flag():
    from normalize import drop_nukta, normalize

    assert drop_nukta(normalize("ज़रूर")) == drop_nukta(normalize("जरूर"))
    assert normalize("ज़रूर") != normalize("जरूर")  # primary score keeps it
    rows = [{"run": "a", "dataset": "d", "wer": 0.20, "wer_nukta_ignored": 0.19},
            {"run": "b", "dataset": "d", "wer": 0.21, "wer_nukta_ignored": 0.17},
            {"run": "c", "dataset": "e", "wer": 0.30, "wer_nukta_ignored": None}]
    flips = harness.rank_flips(rows)
    assert len(flips) == 1 and "a" in flips[0] and "b" in flips[0]


def test_busy_lock_is_taken_and_released_and_waits_for_gate_and_memory(tmp_path, monkeypatch):
    from wbeval import common

    gate, busy = tmp_path / "gate.lock", tmp_path / "busy.lock"
    monkeypatch.setattr(common, "GATE_LOCK", gate)
    monkeypatch.setattr(common, "BUSY_LOCK", busy)
    free = iter([1.0, 10.0, 10.0])
    monkeypatch.setattr(common, "free_ram_gb", lambda: next(free))
    common.WAIT_LOG.clear()
    gate.write_text("track A", encoding="utf-8")
    checks = {"n": 0}

    def fake_sleep(_):
        checks["n"] += 1
        if checks["n"] > 5:  # never spin: a wrong path must fail fast
            raise AssertionError("waited more than 5 times: a lock path is not isolated")
        gate.unlink(missing_ok=True)  # Track A finishes during the first wait

    monkeypatch.setattr(common.time, "sleep", fake_sleep)
    seen = {}
    proc = common.run_heavy([sys.executable, "-c", "import sys; print(open(sys.argv[1]).read())", str(busy)],
                            "test step", capture_output=True, text=True)
    seen["inside"] = proc.stdout
    assert "step: test step" in seen["inside"] and f"pid: {__import__('os').getpid()}" in seen["inside"]
    assert not busy.exists(), "busy lock must be removed after the step"
    kinds = [w["kind"] for w in common.WAIT_LOG]
    assert kinds == ["gate-lock", "memory"]  # waited for Track A, then for RAM (1 GB < 6 GB)
    assert checks["n"] >= 2


def test_stale_busy_lock_from_a_dead_process_is_replaced(tmp_path, monkeypatch):
    from wbeval import common

    busy = tmp_path / "busy.lock"
    busy.write_text("step: old\npid: 999999\nstarted: x\n", encoding="utf-8")
    monkeypatch.setattr(common, "BUSY_LOCK", busy)
    assert common._take_busy_lock("new")
    common._release_busy_lock()
    assert not busy.exists()


def test_gate_request_flag_blocks_the_busy_lock(tmp_path, monkeypatch):
    from wbeval import common

    flag, gate, busy = tmp_path / "request.flag", tmp_path / "gate.lock", tmp_path / "busy.lock"
    monkeypatch.setattr(common, "GATE_REQUEST", flag)
    monkeypatch.setattr(common, "GATE_LOCK", gate)
    monkeypatch.setattr(common, "BUSY_LOCK", busy)
    monkeypatch.setattr(common, "free_ram_gb", lambda: 20.0)
    flag.write_text("track A wants the machine", encoding="utf-8")
    seen = []

    def fake_sleep(_):
        if len(seen) > 5:  # never spin: a wrong path must fail fast
            raise AssertionError("waited more than 5 times: a lock path is not isolated")
        seen.append(busy.exists())  # the busy lock must not exist while the flag is up
        flag.unlink(missing_ok=True)

    monkeypatch.setattr(common.time, "sleep", fake_sleep)
    common.WAIT_LOG.clear()
    common.acquire_heavy_slot("test step")
    assert seen == [False] and busy.exists()
    assert [w["kind"] for w in common.WAIT_LOG] == ["gate-request"]
    common.release_heavy_slot()
    assert not busy.exists()


def test_unit_tests_never_see_track_a_lock_files():
    """conftest.isolated_locks points every lock and flag at a temporary folder: a real
    stelmar-gate-request.flag or stelmar-gate.lock (Track A running gates) cannot make a unit test wait
    or spin, and no test can create the real busy lock."""
    import conftest
    from wbeval import common

    for name, real in conftest.REAL_LOCKS.items():
        isolated = getattr(common, name)
        assert isolated != real and isolated.name == real.name and not isolated.exists(), name
    assert not common.GATE_REQUEST.exists() and not common.GATE_LOCK.exists()


def test_detect_fallback_never_chooses_english():
    from wbeval.engines import choose_language

    assert choose_language("ur", ["en", "hi"]) == ("hi", "ur")  # hi <-> ur confusion stays Hindi
    assert choose_language(None, ["en", "hi"]) == ("hi", None)
    assert choose_language("hi", ["en", "hi"]) == ("hi", None)
    assert choose_language("en", ["en", "hi"]) == ("en", None)
    assert choose_language("fr", ["en"]) == ("en", "fr")  # only English expected


def test_detection_outside_the_set_counts_as_a_miss():
    rec = {"id": "a", "lang": "hi", "mixed": False, "duration_s": 1.0, "ref": "नमस्ते", "hyp": "नमस्ते",
           "segments": [{"start_ms": 0, "end_ms": 1000, "lang": "hi"}], "elapsed_s": 1.0,
           "detected_outside_set": "ur"}
    s = harness.summarise([rec], lang_mode="detect")
    assert s["lang_detection"]["accuracy"] == 0.0 and s["lang_detection"]["confusion"] == {"hi->ur": 1}


def test_truncation_guard():
    import metrics

    words = " ".join(f"शब्द{i}" for i in range(40))
    assert metrics.truncated(metrics.prepare("a", words, " ".join(words.split()[:20]), "hi", False))
    assert not metrics.truncated(metrics.prepare("b", words, words, "hi", False))
    gap = " ".join(words.split()[:30] + words.split()[32:])  # 2 words missing: not truncated
    assert not metrics.truncated(metrics.prepare("c", words, gap, "hi", False))
    run = " ".join(words.split()[:2] + ["x"] * 13 + words.split()[38:])  # 23 in a row replaced/deleted
    assert metrics.truncated(metrics.prepare("d", words, run, "hi", False))


def test_rank_check_leaves_out_runs_trained_on_the_set():
    rows = [{"run": "a", "dataset": "d", "wer": 0.20, "wer_nukta_ignored": 0.19, "independent": False},
            {"run": "b", "dataset": "d", "wer": 0.21, "wer_nukta_ignored": 0.17}]
    assert harness.rank_flips(rows) == []


def test_vad_output_parsing_and_units():
    from wbeval.chunking import parse_vad_output

    out = ("Detected 2 speech segments:\nSpeech segment 0: start = 50.00, end = 820.00\n"
           "Speech segment 1: start = 900.00, end = 1450.00\n")
    assert parse_vad_output(out, 15.0) == [(500, 8200), (9000, 14500)]  # centiseconds
    assert parse_vad_output("Speech segment 0: start = 0.50, end = 8.20\n", 15.0) == [(500, 8200)]
    with pytest.raises(ValueError):
        parse_vad_output("Speech segment 0: start = 1.0, end = 99999.0\n", 10.0)


def test_chunks_stay_within_the_limit_and_cut_at_pauses():
    from wbeval.chunking import fixed_chunks, merge_segments

    segs = [(0, 4000), (4500, 9000), (9800, 14000), (15000, 21000), (22000, 56000)]
    chunks = merge_segments(segs, 15.0, 56000, pad_ms=0)
    assert all(c.seconds <= 15.0 for c in chunks)
    assert {0, 15000}.issubset({c.start_ms for c in chunks})  # cut at the pauses, not mid-speech
    assert chunks[-1].end_ms == 56000
    assert sum(1 for c in chunks if c.start_ms >= 22000) == 3  # the 34 s segment is split in three
    fixed = fixed_chunks(56000, 15.0)
    assert len(fixed) == 4 and all(c.seconds <= 15.0 for c in fixed)


def test_unknown_independence_is_reported_and_not_ranked(tmp_path, monkeypatch):
    data_dir = make_set(tmp_path, "coshe-eval", [("a", "one two", "en", False)])
    fx = tmp_path / "fixture"
    write_fixture(fx, "a", [Segment(0, 1000, "one two", "en")])
    entries = {"hinglish-q8_0": {"file": "x", "sha256": "s", "candidate": "hinglish"},
               "silero-v6.2.0": {"file": "v", "sha256": "v"}}
    monkeypatch.setattr(harness.manifest, "find", lambda mid: entries[mid])
    s = harness.run("wb-engine", Setup("hinglish-q8_0", {"*": "hinglish-q8_0"}), "coshe-eval", "night",
                    fixture=str(fx), data_dir=data_dir, results_dir=tmp_path / "r")
    # candidates.toml (committed) says Hinglish is independence unknown on coshe-eval
    assert s["independent"] is None and s["independence_unknown_models"] == ["hinglish-q8_0"]
    assert s["not_independent_models"] == []
    table = harness.write_tables(tmp_path / "r", tmp_path / "none.toml").read_text(encoding="utf-8")
    assert "**unknown** (hinglish-q8_0)" in table
    rows = [{"run": "a", "dataset": "d", "wer": 0.20, "wer_nukta_ignored": 0.19, "independent": None},
            {"run": "b", "dataset": "d", "wer": 0.21, "wer_nukta_ignored": 0.17}]
    assert harness.rank_flips(rows) == []
