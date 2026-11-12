"""Evaluation harness: run a model setup over a dataset with a role's settings and score it.

    # one run
    eval/.venv/Scripts/python eval/run.py run --setup medium --dataset fleurs-hi --role rolling \
        --engine whisper-cli --limit 20
    # every run in eval/runs.toml, then every table (skips runs that are already complete)
    eval/.venv/Scripts/python eval/run.py all
    # tables only, from the summaries on disk
    eval/.venv/Scripts/python eval/run.py tables
    # record wb-engine replay fixtures from whisper-cli (used until Phase 2 is merged); keep the
    # recorded dataset output out of git
    eval/.venv/Scripts/python eval/run.py record-fixture --setup medium --dataset fleurs-hi --limit 3 \
        --out eval/results/fixtures/fleurs-hi

Engines: `wb-engine` (`batch --file`; set WB_ENGINE to the binary, or pass --engine-fixture to
replay recorded events) and `whisper-cli` (whisper.cpp's CLI, the reference path).

Outputs (git-ignored) in eval/results/<run>/: utterances.jsonl (one line per utterance, resumable)
and summary.json (WER, CER for Hindi and Urdu, English words written in Devanagari, script mix,
per-chunk language detection, speed, the whisper.cpp version, model hashes and machine).
eval/results/TABLES.md collects every summary. Heavy steps wait for the Track A gate lock and run at
below-normal priority.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import logging
import os
import platform
import sys
import tomllib
from dataclasses import asdict, fields
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402
from wbeval import manifest  # noqa: E402
from wbeval.common import (  # noqa: E402
    DATA_DIR, EVAL_DIR, REPO, RESULTS_DIR, WAIT_LOG, rel, set_below_normal_priority, setup_logging,
)
from wbeval.data import Utterance, load_manifest, read_sources  # noqa: E402
from wbeval.engines import (  # noqa: E402
    Engine, Settings, Setup, WbEngine, WhisperCli, WhisperCliOnEngineChunks, segments_to_events,
)
from wbeval.tools import whisper_cpp_info  # noqa: E402

log = logging.getLogger("run")
ROLES_FILE = EVAL_DIR / "roles.toml"
RUNS_FILE = EVAL_DIR / "runs.toml"
CER_LANGS = {"hi", "ur"}


# ----------------------------------------------------------------------------------------- configuration

def load_toml(path: Path) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def role_settings(role: str, threads: int | None = None, glossary: str | None = None) -> Settings:
    roles = load_toml(ROLES_FILE)
    if role not in roles:
        raise SystemExit(f"unknown role {role!r}; roles: {', '.join(roles)}")
    known = {f.name for f in fields(Settings)}
    unknown = sorted(set(roles[role]) - known)
    if unknown:
        raise SystemExit(f"roles.toml [{role}]: unknown setting(s) {unknown}; known: {sorted(known - {'role'})}")
    values = dict(roles[role])
    s = Settings(role=role, **values)
    if threads:
        s.threads = threads
    if glossary is not None:
        s.glossary = glossary
    return s


def parse_setup(spec: str, setups: dict) -> Setup:
    """A named setup from runs.toml, a model id, or `en=id,hi=id` routing."""
    if spec in setups:
        cfg = setups[spec]
        return Setup(spec, dict(cfg["models"]), cfg.get("detector"))
    if "=" in spec:
        models = dict(part.split("=", 1) for part in spec.split(","))
        return Setup(spec.replace("=", "-").replace(",", "+"), models)
    return Setup(spec, {"*": spec})


def make_engine(name: str, fixture: str | None) -> Engine:
    if name == "whisper-cli":
        return WhisperCli()
    if name == "wb-engine":
        return WbEngine(fixture_dir=Path(fixture) if fixture else None)
    if name == "whisper-cli-engine-chunks":
        return WhisperCliOnEngineChunks()
    raise SystemExit(f"unknown engine {name!r}")


def clip_list_ids(path: str, dataset: str) -> list[str]:
    """Ids of `dataset` in a committed clip list (`dataset<TAB>id` per line, # comments)."""
    ids = []
    for line in (REPO / path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            ds, _, uid = line.partition("	")
            if ds == dataset:
                ids.append(uid)
    return ids


def limit_key(limit: int | str | None) -> int | str | None:
    """What a limit contributes to the fingerprint: a clip list counts by its content."""
    if isinstance(limit, str):
        return "clips:" + hashlib.sha256((REPO / limit).read_bytes()).hexdigest()
    return limit


def select(utts: list[Utterance], limit: int | str | None, dataset: str | None = None) -> list[Utterance]:
    """Deterministic spread over the sorted set: every k-th utterance. A string limit is the path of a
    committed clip list (eval/clips/*.txt); exactly those ids are used, and a missing id is an error."""
    utts = sorted(utts, key=lambda u: u.id)
    if isinstance(limit, str):
        by_id = {u.id: u for u in utts}
        ids = clip_list_ids(limit, dataset or "")
        missing = [i for i in ids if i not in by_id]
        if not ids or missing:
            raise SystemExit(f"clip list {limit}: {len(ids)} ids for {dataset}, missing from the manifest: {missing[:3]}")
        return [by_id[i] for i in ids]
    if not limit or limit >= len(utts):
        return utts
    step = len(utts) / limit
    return [utts[int(i * step)] for i in range(limit)]


def machine_info(name: str | None) -> dict:
    cpu = platform.processor()
    if sys.platform == "win32":
        try:
            import winreg

            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0") as k:
                cpu = winreg.QueryValueEx(k, "ProcessorNameString")[0].strip()
        except OSError:
            pass
    return {"name": name or os.environ.get("WB_MACHINE", "unnamed"), "cpu": cpu, "logical_cpus": os.cpu_count(),
            "emulation": os.environ.get("WB_EMULATE", "none"), "os": platform.platform(terse=True)}


def run_name(engine: str, setup: Setup, dataset: str, role: str, limit: int | str | None, lang_mode: str) -> str:
    tag = f"__n{limit}" if isinstance(limit, int) else ("__" + Path(limit).stem if limit else "")
    return f"{dataset}__{setup.name}__{role}__{lang_mode}__{engine}" + tag


def fingerprint(engine: dict, setup: Setup, settings: Settings, models: dict, dataset_sha: str | None,
                lang_mode: str, langs: list[str] | None, limit: int | str | None, machine: str | None = None) -> str:
    """Hash of everything that changes a run's output. A run on disk with another fingerprint is stale:
    `all` re-runs it, and `run` refuses to resume it without --force."""
    wcpp = whisper_cpp_info()
    key = {"engine": engine,
           "whisper_cpp": wcpp.get("commit"), "setup": setup.models, "detector": setup.detector,
           "settings": asdict(settings), "models": {k: v.get("sha256") for k, v in models.items()},
           "dataset": dataset_sha, "lang_mode": lang_mode, "langs": langs, "limit": limit_key(limit),
           # Speed numbers belong to a machine and emulation profile: a summary from another one is stale.
           "machine": {k: v for k, v in machine_info(machine).items() if k in ("name", "emulation")}}
    return hashlib.sha256(json.dumps(key, sort_keys=True).encode("utf-8")).hexdigest()


def language_source(lang_mode: str, set_langs: set[str]) -> str:
    """Where the decoding language came from. 'oracle' only when the set mixes languages (CoSHE-Eval:
    en or hi per utterance, taken from the reference), not for a single-language set."""
    if lang_mode == "detect":
        return "detected"
    return "given: oracle, from the reference" if len(set_langs) > 1 else "given: the set's single language"


def not_independent(models: dict, dataset: str) -> list[str]:
    """Models of the setup that were trained on (a split of) this evaluation set."""
    return [mid for mid in models if dataset in manifest.training_flags(mid)["not_independent_on"]]


def independence_unknown(models: dict, dataset: str) -> list[str]:
    """Models whose sources do not say whether this evaluation set was used for their training."""
    return [mid for mid in models if dataset in manifest.training_flags(mid)["independence_unknown_on"]]


def independence_fields(models: dict, dataset: str) -> dict:
    """`independent`: False (trained on the set), None (unknown) or True; plus the models behind it."""
    leaked, unknown = not_independent(models, dataset), independence_unknown(models, dataset)
    return {"independent": False if leaked else (None if unknown else True),
            "not_independent_models": leaked, "independence_unknown_models": unknown}


def expected_fingerprint(engine_name: str, setup: Setup, dataset: str, role: str, limit: int | str | None,
                         lang_mode: str, langs: list[str] | None, threads: int | None, glossary: str | None,
                         fixture: str | None, data_dir: Path | None = None, machine: str | None = None) -> str:
    settings = role_settings(role, threads, glossary)
    return fingerprint(make_engine(engine_name, fixture).describe(), setup, settings, model_hashes(setup, settings),
                       dataset_hash(dataset, data_dir or DATA_DIR), lang_mode, langs, limit, machine)


# --------------------------------------------------------------------------------------------- running

def run(engine_name: str, setup: Setup, dataset: str, role: str, limit: int | str | None = None,
        lang_mode: str = "fixed", langs: list[str] | None = None, threads: int | None = None,
        glossary: str | None = None, fixture: str | None = None, force: bool = False,
        machine: str | None = None, data_dir: Path = DATA_DIR, results_dir: Path = RESULTS_DIR,
        name: str | None = None) -> dict:
    settings = role_settings(role, threads, glossary)
    engine = make_engine(engine_name, fixture)
    utts = select(load_manifest(dataset, data_dir), limit, dataset)
    name = name or run_name(engine_name, setup, dataset, role, limit, lang_mode)
    out_dir = results_dir / name
    out_dir.mkdir(parents=True, exist_ok=True)
    utt_path = out_dir / "utterances.jsonl"
    engine_desc = engine.describe()
    hashes = model_hashes(setup, settings)
    dataset_sha = dataset_hash(dataset, data_dir)
    fp = fingerprint(engine_desc, setup, settings, hashes, dataset_sha, lang_mode, langs, limit, machine)
    fp_path = out_dir / "fingerprint.txt"
    old_fp = fp_path.read_text(encoding="utf-8").strip() if fp_path.exists() else None
    if utt_path.exists() and old_fp != fp and not force:
        raise SystemExit(f"{name}: results on disk were made with other settings, models, data or whisper.cpp "
                         "(fingerprint differs); re-run with --force")
    if force:
        utt_path.unlink(missing_ok=True)
        (out_dir / "summary.json").unlink(missing_ok=True)
    fp_path.write_text(fp + "\n", encoding="utf-8")
    done = {}
    if utt_path.exists():
        for line in utt_path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                rec = json.loads(line)
                if "error" not in rec:
                    done[rec["id"]] = rec
    set_below_normal_priority()
    log.info("%s: %d utterances (%d already done)", name, len(utts), len(done))
    set_path = data_dir / dataset
    with utt_path.open("a", encoding="utf-8") as f:
        for i, u in enumerate(utts, 1):
            if u.id in done:
                continue
            expected = langs or [u.lang]
            WAIT_LOG.clear()
            try:
                t = engine.transcribe(u.wav_path(set_path), setup, expected if lang_mode == "detect" else [u.lang],
                                      lang_mode, settings, u.id)
            except Exception as e:  # record and continue; failures are counted in the summary
                log.error("%s: %s", u.id, e)
                f.write(json.dumps({"id": u.id, "error": str(e)[:500]}, ensure_ascii=False) + "\n")
                f.flush()
                continue
            rec = {"id": u.id, "lang": u.lang, "mixed": u.mixed, "duration_s": u.duration_s, "ref": u.text,
                   "hyp": t.text, "segments": [asdict(s) for s in t.segments], "models": sorted(set(t.model_by_segment)),
                   "detected_lang": t.detected_lang, "detected_prob": t.detected_prob,
                   "detected_outside_set": t.detected_outside_set, "chunks": t.chunks,
                   "elapsed_s": round(t.elapsed_s, 3),
                   # gate-lock, memory and busy-lock waits before this utterance's engine calls (not in elapsed_s)
                   "waits": list(WAIT_LOG)}
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
            f.flush()
            done[u.id] = rec
            if i % 10 == 0:
                log.info("%s: %d/%d", name, i, len(utts))
    records = [done[u.id] for u in utts if u.id in done]
    summary = summarise(records, failures=len(utts) - len(records), lang_mode=lang_mode)
    summary.update(
        run=name, fingerprint=fp, engine=engine_desc, setup={"name": setup.name, "models": setup.models,
                                                             "detector": setup.detector},
        **independence_fields(hashes, dataset),
        language_source=language_source(lang_mode, {u.lang for u in utts}),
        models=hashes, dataset=dataset, dataset_manifest_sha256=dataset_sha,
        role=role, settings=asdict(settings), lang_mode=lang_mode, expected_langs=langs, limit=limit,
        whisper_cpp=whisper_cpp_info(), machine=machine_info(machine),
        speed_method="whisper-cli wall time per clip, one clip at a time; rolling, night and live reload the model "
                     "for every chunk (one process per chunk), near-live loads it once per clip; gate-lock, memory "
                     "and busy-lock waits excluded and listed under waits",
        waits=[{"utterance": r["id"], **w} for r in records for w in r.get("waits", [])],
        wait_total_s=round(sum(w["seconds"] for r in records for w in r.get("waits", [])), 1),
        finished_at=dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
    )
    (out_dir / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    log.info("%s: WER %s CER %s", name, fmt_pct(summary["wer"]), fmt_pct(summary["cer"]))
    return summary


def model_hashes(setup: Setup, settings: Settings) -> dict:
    ids = set(setup.models.values()) | ({setup.detector} if setup.detector else set())
    if settings.vad:
        ids.add(settings.vad_model)
    out = {}
    for mid in sorted(ids):
        try:
            m = manifest.find(mid)
            out[mid] = {"file": m["file"], "sha256": m["sha256"], "quant": m.get("quant")}
        except KeyError:
            out[mid] = {"file": None, "sha256": None}
    return out


def dataset_hash(dataset: str, data_dir: Path) -> str | None:
    for s in read_sources(data_dir):
        if s.set == dataset:
            return s.manifest_sha256
    return None


def summarise(records: list[dict], failures: int = 0, lang_mode: str = "detect") -> dict:
    scored = [metrics.prepare(r["id"], r["ref"], r["hyp"], r["lang"], r["mixed"]) for r in records]
    langs = {r["lang"] for r in records}
    detection = metrics.LangDetection()
    for r in records:
        for s in r["segments"]:
            # A detection outside the session's languages is a miss even though the clip was then
            # decoded in an expected language.
            detected = r.get("detected_outside_set") or s["lang"]
            detection.add(r["lang"], detected, (s["end_ms"] - s["start_ms"]) / 1000)
    latin = metrics.latin_words(scored)
    audio_s = sum(r["duration_s"] for r in records)
    elapsed = sum(r["elapsed_s"] for r in records)
    return {
        "utterances": len(records), "failures": failures,
        "empty_references": sum(1 for s in scored if not s.ref),
        "wer": metrics.wer(scored),
        "cer": metrics.cer(scored) if langs & CER_LANGS else None,
        # Secondary Hindi scores with the nukta ignored (eval/normalize.py); None without Hindi.
        "wer_nukta_ignored": metrics.wer(metrics.nukta_ignored(scored)) if "hi" in langs else None,
        "cer_nukta_ignored": metrics.cer(metrics.nukta_ignored(scored)) if "hi" in langs else None,
        "latin_words": {**asdict(latin), "devanagari_rate": latin.devanagari_rate},
        "script_mix": metrics.script_mix(scored),
        "wrong_script_rate": metrics.wrong_script_rate(scored),
        # U+FFFD in the output means a token ended inside a multi-byte character (broken decoding).
        "broken_utf8_utterances": sum(1 for r in records if "\ufffd" in r["hyp"]),
        # Speech lost: hypothesis/reference words under 0.7, or more than 20 reference words deleted in a row.
        "truncated_utterances": sum(1 for s in scored if metrics.truncated(s)),
        "longest_chunk_s": max((round((b - a) / 1000, 1) for r in records for a, b in r.get("chunks", [])),
                               default=None),
        # In fixed mode the engine is told the language, so every segment carries it: not a measurement.
        "lang_detection": {
            "chunks": detection.chunks, "accuracy": detection.accuracy, "accuracy_by_time": detection.accuracy_by_time,
            "confusion": {f"{e}->{d}": n for (e, d), n in sorted(detection.confusion.items())},
            "detected_outside_set": sum(1 for r in records if r.get("detected_outside_set")),
            "reference": "vs utterance language from the dataset manifest (code-switched sets: matrix language), "
                         "until Phase 5 adds chunk labels",
        } if lang_mode == "detect" else {"chunks": detection.chunks, "accuracy": None, "accuracy_by_time": None,
                                         "confusion": {}, "reference": "not measured: language given (fixed mode)"},
        "audio_s": round(audio_s, 2), "elapsed_s": round(elapsed, 2),
        "rtf": round(elapsed / audio_s, 4) if audio_s else None,
    }


# ---------------------------------------------------------------------------------------------- tables

def fmt_pct(x: float | None) -> str:
    return "n/a" if x is None else f"{100 * x:.1f}"


def rank_flips(summaries: list[dict]) -> list[str]:
    """Datasets where ranking runs by WER and by WER with the nukta ignored disagree."""
    out = []
    by_set: dict[str, list[dict]] = {}
    for s in summaries:
        # Runs on a set the model was trained on are never ranked, so they are left out here too.
        if s.get("independent", True) is True and s.get("wer") is not None and s.get("wer_nukta_ignored") is not None:
            by_set.setdefault(s["dataset"], []).append(s)
    for dataset, rows in sorted(by_set.items()):
        for i, a in enumerate(rows):
            for b in rows[i + 1:]:
                primary = (a["wer"] > b["wer"]) - (a["wer"] < b["wer"])
                secondary = (a["wer_nukta_ignored"] > b["wer_nukta_ignored"]) - (a["wer_nukta_ignored"] < b["wer_nukta_ignored"])
                if primary and secondary and primary != secondary:
                    out.append(f"{dataset}: {a['run']} ({fmt_pct(a['wer'])} / {fmt_pct(a['wer_nukta_ignored'])}) and "
                               f"{b['run']} ({fmt_pct(b['wer'])} / {fmt_pct(b['wer_nukta_ignored'])}) swap places")
    return out


def write_tables(results_dir: Path = RESULTS_DIR, runs_file: Path = RUNS_FILE) -> Path:
    summaries = []
    for p in sorted(results_dir.glob("*/summary.json")):
        summaries.append(json.loads(p.read_text(encoding="utf-8")))
    by_name = {s["run"]: s for s in summaries}
    lines = [
        "# Evaluation tables", "",
        f"Generated by `eval/run.py tables` on {dt.datetime.now(dt.timezone.utc):%Y-%m-%d %H:%M} UTC from "
        f"{len(summaries)} run summaries. WER and CER in %, script-aware normalisation (`eval/normalize.py`).",
        "Latin→Devanagari: share of the reference's Latin-script words (English, plus a little romanised Hindi in",
        "CoSHE-Eval) that came out in Devanagari; the number of such reference words is in brackets (a rate on",
        "a handful of words means little). Broken: utterances with U+FFFD in the output. Truncated:",
        "utterances that lost speech (hypothesis under 0.7 words per reference word, or over 20 reference words",
        "deleted in a row). Longest chunk: the longest piece decoded in one go (should stay at or under 15 s).",
        "Independent: **no** when a model of the setup was trained on this set's corpus, **unknown** when its",
        "sources do not say (neither is evidence or ranked; Phase 5 needs a clean set or a statement). Language: `fixed` = given to the engine (detection not measured; `oracle`",
        "when taken from a mixed set's reference), `detect` = detected (accuracy vs the utterance's language).",
        "RTF here is whisper-cli wall time per clip. Rolling, night and live run one whisper-cli process per chunk, so the model is reloaded for every chunk; near-live runs one process per clip. RTFs of different roles are therefore not comparable with each other or with wb-engine, which keeps the model loaded; speed decisions use `wb-engine bench` (Phase 2), not this column. Gate-lock, memory and busy-lock waits are excluded.", "",
        "WER and CER keep the nukta (ज़ ≠ ज; primary, used by every choice rule); WER (nukta ignored) is the",
        "secondary Hindi score with U+093C removed. See the nukta rank check below the table.", "",
        "| Run | Dataset | Setup | Role | Language | Engine | n | WER | WER (nukta ignored) | CER | Latin→Devanagari | "
        "Wrong script | Broken | Truncated | Longest chunk (s) | Lang acc. | Independent | RTF | Threads | Machine | "
        "whisper.cpp build |",
        "| --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: "
        "| --- | ---: | ---: | --- | --- |",
    ]
    for s in summaries:
        eng = s["engine"]
        eng_label = eng["engine"] + (" (replay)" if eng.get("mode") == "replay" else "")
        wcpp = s.get("whisper_cpp", {})
        flags = wcpp.get("cmake_flags", [])
        build = "portable AVX2" if "-DGGML_NATIVE=OFF" in flags else "build flags not recorded"
        language = s["lang_mode"] + (" (oracle)" if "oracle" in s.get("language_source", "") else "")
        if s.get("independent", True) is False:
            independent = f"**no** ({', '.join(s.get('not_independent_models', []))})"
        elif s.get("independent") is None:
            independent = f"**unknown** ({', '.join(s.get('independence_unknown_models', []))})"
        else:
            independent = "yes"
        lines.append(
            f"| {s['run']} | {s['dataset']} | {s['setup']['name']} | {s['role']} | {language} | {eng_label} | "
            f"{s['utterances']}{' (' + str(s['failures']) + ' failed)' if s['failures'] else ''} | "
            f"{fmt_pct(s['wer'])} | {fmt_pct(s.get('wer_nukta_ignored'))} | {fmt_pct(s['cer'])} | "
            f"{fmt_pct(s['latin_words']['devanagari_rate'])} ({s['latin_words']['ref_latin_words']}) | "
            f"{fmt_pct(s['wrong_script_rate'])} | {s.get('broken_utf8_utterances', 'n/a')} | "
            f"{s.get('truncated_utterances', 'n/a')} | {s.get('longest_chunk_s') or 'n/a'} | "
            f"{fmt_pct(s['lang_detection']['accuracy'])} | {independent} | "
            f"{s['rtf'] if s['rtf'] is not None else 'n/a'} | {s['settings']['threads']} | "
            f"{s['machine']['name']} ({s['machine']['emulation']}) | "
            f"{wcpp.get('tag', '?')} ({wcpp.get('commit', '?')[:12]}), {build} |"
        )
    flips = rank_flips(summaries)
    lines += ["", "## Nukta rank check", "",
              "Datasets where ranking by WER (nukta kept) and by WER (nukta ignored) disagree; Phase 5 looks at each.",
              ""] + ([f"- **Flag:** {f}" for f in flips] or ["- None: both scores rank every pair of runs the same way."])
    comparisons = load_toml(runs_file).get("compare", []) if runs_file.exists() else []
    if comparisons:
        lines += ["", "## Comparisons", "",
                  "| A | B | WER A | WER B | Difference (points) | Limit | Result | whisper.cpp A / B |",
                  "| --- | --- | ---: | ---: | ---: | ---: | --- | --- |"]
        for c in comparisons:
            a, b = by_name.get(c["a"]), by_name.get(c["b"])
            if not a or not b or a["wer"] is None or b["wer"] is None:
                lines.append(f"| {c['a']} | {c['b']} | | | | {c['max_wer_points']} | not run yet | |")
                continue
            diff = 100 * abs(a["wer"] - b["wer"])
            va = a.get("whisper_cpp", {}).get("tag", "?")
            vb = b.get("engine", {}).get("whisper_cpp", {}).get("tag") or b.get("engine", {}).get("version", "?")
            lines.append(f"| {c['a']} | {c['b']} | {fmt_pct(a['wer'])} | {fmt_pct(b['wer'])} | {diff:.2f} | "
                         f"{c['max_wer_points']} | {('PASS' if diff <= c['max_wer_points'] else 'FAIL') if not c.get('informational') else 'information only (not a gate)'} | {va} / {vb} |")
    path = results_dir / "TABLES.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


# ---------------------------------------------------------------------------------------------- commands

def cmd_all(args) -> int:
    cfg = load_toml(RUNS_FILE)
    setups = cfg.get("setups", {})
    failed = 0
    for spec in cfg.get("run", []):
        if args.only and args.only not in json.dumps(spec):
            continue
        if spec.get("needs") == "phase-2" and not os.environ.get("WB_ENGINE") and not spec.get("fixture"):
            log.info("skipping %s: needs wb-engine from Phase 2 (set WB_ENGINE)", spec.get("name", spec))
            continue
        setup = parse_setup(spec["setup"], setups)
        name = spec.get("name")
        summary_path = RESULTS_DIR / (name or run_name(spec["engine"], setup, spec["dataset"], spec["role"],
                                                       spec.get("limit"), spec.get("lang_mode", "fixed"))) / "summary.json"
        if not (DATA_DIR / spec["dataset"] / "manifest.jsonl").exists():
            log.warning("skipping %s: dataset %s not prepared", name or spec, spec["dataset"])
            continue
        force = args.force
        if summary_path.exists() and not force:
            try:
                current = expected_fingerprint(spec["engine"], setup, spec["dataset"], spec["role"], spec.get("limit"),
                                               spec.get("lang_mode", "fixed"), spec.get("langs"), args.threads,
                                               spec.get("glossary"), spec.get("fixture"), machine=args.machine)
            except Exception:
                log.exception("cannot check %s", rel(summary_path))
                failed += 1
                continue
            if json.loads(summary_path.read_text(encoding="utf-8")).get("fingerprint") == current:
                log.info("up to date: %s", rel(summary_path))
                continue
            log.info("stale (settings, models, data or whisper.cpp changed): re-running %s", rel(summary_path))
            force = True
        try:
            run(spec["engine"], setup, spec["dataset"], spec["role"], spec.get("limit"), spec.get("lang_mode", "fixed"),
                spec.get("langs"), args.threads, spec.get("glossary"), spec.get("fixture"), force,
                args.machine, name=name)
        except Exception:
            log.exception("run failed: %s", spec)
            failed += 1
    log.info("tables: %s", rel(write_tables()))
    return 1 if failed else 0


def cmd_rescore(args) -> int:
    """Recompute the scores of finished runs from their utterances.jsonl (no decoding): picks up
    scoring changes such as the nukta-ignored column and the independence flags."""
    for summary_path in sorted(RESULTS_DIR.glob("*/summary.json")):
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
        utt_path = summary_path.with_name("utterances.jsonl")
        if not utt_path.exists():
            continue
        records = [r for line in utt_path.read_text(encoding="utf-8").splitlines() if line.strip()
                   and "error" not in (r := json.loads(line))]
        summary.update(summarise(records, failures=summary.get("failures", 0), lang_mode=summary["lang_mode"]))
        summary.update(**independence_fields(summary.get("models", {}), summary["dataset"]),
                       language_source=language_source(summary["lang_mode"], {r["lang"] for r in records}),
                       rescored_at=dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"))
        summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        log.info("rescored %s", rel(summary_path))
    log.info("tables: %s", rel(write_tables()))
    return 0


def cmd_record_fixture(args) -> int:
    """Run whisper-cli and save its segments as wb-engine events, one JSONL file per utterance."""
    setups = load_toml(RUNS_FILE).get("setups", {}) if RUNS_FILE.exists() else {}
    setup = parse_setup(args.setup, setups)
    settings = role_settings(args.role, args.threads)
    engine = WhisperCli()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    set_below_normal_priority()
    for u in select(load_manifest(args.dataset), args.limit, args.dataset):
        t = engine.transcribe(u.wav_path(DATA_DIR / args.dataset), setup, [u.lang], "fixed", settings, u.id)
        (out / f"{u.id}.jsonl").write_text(segments_to_events(t, setup.model_for(u.lang)), encoding="utf-8")
        log.info("fixture %s: %s", u.id, rel(out / f"{u.id}.jsonl"))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--machine", help="machine name for the summary (default: WB_MACHINE or 'unnamed')")
    parser.add_argument("--threads", type=int, help="override the role's thread count")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("run", help="one setup over one dataset")
    p.add_argument("--setup", required=True, help="setup name from runs.toml, a model id, or en=id,hi=id")
    p.add_argument("--dataset", required=True)
    p.add_argument("--role", default="rolling")
    p.add_argument("--engine", default="whisper-cli", choices=["whisper-cli", "wb-engine"])
    p.add_argument("--engine-fixture", help="replay recorded wb-engine events from this folder")
    p.add_argument("--limit", type=int)
    p.add_argument("--lang-mode", default="fixed", choices=["fixed", "detect"])
    p.add_argument("--langs", help="expected session languages for detection, e.g. en,hi")
    p.add_argument("--glossary")
    p.add_argument("--name", help="run folder name (default: derived from the arguments)")
    p.add_argument("--force", action="store_true", help="discard earlier results of this run")

    p = sub.add_parser("all", help="every run in runs.toml, then the tables")
    p.add_argument("--only", help="only runs whose spec contains this text")
    p.add_argument("--force", action="store_true")

    sub.add_parser("tables", help="regenerate eval/results/TABLES.md")
    sub.add_parser("rescore", help="recompute scores of finished runs from their utterances (no decoding)")

    p = sub.add_parser("record-fixture", help="record wb-engine replay fixtures with whisper-cli")
    p.add_argument("--setup", required=True)
    p.add_argument("--dataset", required=True)
    p.add_argument("--role", default="rolling")
    p.add_argument("--limit", type=int, default=3)
    p.add_argument("--out", required=True)

    args = parser.parse_args()
    setup_logging("run")
    if args.cmd == "run":
        setups = load_toml(RUNS_FILE).get("setups", {}) if RUNS_FILE.exists() else {}
        summary = run(args.engine, parse_setup(args.setup, setups), args.dataset, args.role, args.limit, args.lang_mode,
                      args.langs.split(",") if args.langs else None, args.threads, args.glossary, args.engine_fixture,
                      args.force, args.machine, name=args.name)
        log.info("tables: %s", rel(write_tables()))
        return 0 if summary["failures"] == 0 else 1
    if args.cmd == "all":
        return cmd_all(args)
    if args.cmd == "rescore":
        return cmd_rescore(args)
    if args.cmd == "tables":
        log.info("tables: %s", rel(write_tables()))
        return 0
    return cmd_record_fixture(args)


if __name__ == "__main__":
    sys.exit(main())
