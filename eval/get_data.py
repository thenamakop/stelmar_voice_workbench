"""Download the public evaluation sets and prepare them as 16 kHz mono WAV (development time only).

    eval/.venv/Scripts/python eval/get_data.py fleurs-en fleurs-hi fleurs-ur coshe-eval
    eval/.venv/Scripts/python eval/get_data.py kathbath-hi kathbath-ur   # gated: accept terms first
    eval/.venv/Scripts/python eval/get_data.py --list

Files land in eval/data/<set>/ (git-ignored); downloads are cached under HF_HOME. Each set records
its source URL, repository revision, licence, file count and SHA-256 in eval/data/SOURCES.md.
The app never runs this code: it is the only place, with scripts/models/, that downloads anything.
"""

from __future__ import annotations

import argparse
import csv
import io
import logging
import sys
import tarfile
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path

import pyarrow.parquet as pq
from huggingface_hub import HfApi, hf_hub_download
from huggingface_hub.errors import GatedRepoError

sys.path.insert(0, str(Path(__file__).resolve().parent))

from normalize import LATIN, is_mixed, normalize, scripts_in  # noqa: E402
from wbeval import audio  # noqa: E402
from wbeval.common import heavy_step, rel, release_heavy_slot, setup_logging, sha256_file  # noqa: E402
from wbeval.data import Source, SourceFile, Utterance, set_dir, write_source  # noqa: E402

log = logging.getLogger("get_data")
api = HfApi()


def hf_url(repo: str, revision: str, filename: str) -> str:
    return f"https://huggingface.co/datasets/{repo}/resolve/{revision}/{filename}"


def fetch(repo: str, revision: str, filename: str) -> tuple[Path, SourceFile]:
    """Download one dataset file into the HF cache (resumable) and describe it."""
    path = Path(hf_hub_download(repo, filename, repo_type="dataset", revision=revision))
    return path, SourceFile(filename, hf_url(repo, revision, filename), path.stat().st_size, sha256_file(path))


# Repository commits read on 8 November 2026. Change only deliberately: every manifest and result
# records the revision it came from. Kathbath is gated and not downloaded yet: after its first
# download, copy the revision from eval/data/kathbath-hi/source.json into this table.
PINNED_REVISIONS = {
    "google/fleurs": "70bb2e84b976b7e960aa89f1c648e09c59f894dd",
    "soketlabs/CoSHE-Eval": "82809be5178adebee3d993cec875bb22563154aa",
}


def dataset_meta(repo: str) -> tuple[str, str]:
    """Pinned revision and the licence the dataset card declares at that revision."""
    revision = PINNED_REVISIONS.get(repo)
    if revision is None:
        log.warning("%s has no pinned revision; using the current one (pin it in PINNED_REVISIONS)", repo)
    info = api.dataset_info(repo, revision=revision)
    card = info.card_data.to_dict() if info.card_data else {}
    licence = card.get("license", "not declared")
    return info.sha, licence if isinstance(licence, str) else ", ".join(licence)


# --------------------------------------------------------------------------------------------- FLEURS

FLEURS_REPO = "google/fleurs"
FLEURS_LANGS = {"fleurs-en": ("en_us", "en"), "fleurs-hi": ("hi_in", "hi"), "fleurs-ur": ("ur_pk", "ur")}


def prepare_fleurs(name: str) -> None:
    code, lang = FLEURS_LANGS[name]
    revision, licence = dataset_meta(FLEURS_REPO)
    tsv_path, tsv_file = fetch(FLEURS_REPO, revision, f"data/{code}/test.tsv")
    tar_path, tar_file = fetch(FLEURS_REPO, revision, f"data/{code}/audio/test.tar.gz")

    # test.tsv columns: id, file_name, raw_transcription, transcription, phonemes, num_samples, gender
    refs: dict[str, str] = {}
    with tsv_path.open(encoding="utf-8", newline="") as f:
        for row in csv.reader(f, delimiter="\t", quoting=csv.QUOTE_NONE):
            if len(row) >= 3:
                refs[row[1]] = row[2]

    heavy_step(f"resampling {name}")
    out = set_dir(name)
    utts: list[Utterance] = []
    with tarfile.open(tar_path, "r:gz") as tar:
        for member in tar:
            base = Path(member.name).name
            if not member.isfile() or base not in refs:
                continue
            data = tar.extractfile(member)
            if data is None:
                continue
            wav = out / "audio" / (Path(base).stem + ".wav")
            dur = convert_or_reuse(data.read(), wav)
            text = refs[base]
            utts.append(Utterance(Path(base).stem, rel_to(wav, out), text, lang, is_mixed(text), round(dur, 3),
                                  sha256_file(wav)))
    missing = len(refs) - len(utts)
    write_source(Source(
        set=name, title=f"FLEURS test, {code}", url=f"https://huggingface.co/datasets/{FLEURS_REPO}",
        revision=revision, licence=licence, licence_evidence="dataset card metadata; FLEURS paper (CC BY 4.0)",
        terms="Evaluation; redistribution allowed with attribution, but not committed here",
        files=[tsv_file, tar_file],
        notes=f"reference = raw_transcription column; {missing} rows without audio" if missing else
        "reference = raw_transcription column",
    ), utts)
    log.info("%s: %d utterances", name, len(utts))


# ------------------------------------------------------------------------------------------ CoSHE-Eval

COSHE_REPO = "soketlabs/CoSHE-Eval"


def iter_parquet_rows(paths: list[Path], columns: list[str]) -> Iterator[dict]:
    for path in paths:
        pf = pq.ParquetFile(path)
        for batch in pf.iter_batches(batch_size=16, columns=columns):
            yield from batch.to_pylist()


def prepare_coshe(name: str) -> None:
    revision, licence = dataset_meta(COSHE_REPO)
    files = sorted(f for f in api.list_repo_files(COSHE_REPO, repo_type="dataset", revision=revision)
                   if f.endswith(".parquet"))
    if not files:
        raise RuntimeError(f"{COSHE_REPO}: no parquet files at {revision}")
    fetched = [fetch(COSHE_REPO, revision, f) for f in files]
    heavy_step(f"resampling {name}")
    out = set_dir(name)
    utts: list[Utterance] = []
    for row in iter_parquet_rows([p for p, _ in fetched], ["audio_file_name", "audio", "transcription"]):
        stem = Path(row["audio_file_name"] or row["audio"]["path"]).stem
        wav = out / "audio" / f"{stem}.wav"
        dur = convert_or_reuse(row["audio"]["bytes"], wav)
        text = row["transcription"] or ""
        # Matrix language is Hindi; the reference keeps English words in Latin script. Rows whose
        # reference is entirely Latin script are labelled English.
        utts.append(Utterance(stem, rel_to(wav, out), text, coshe_lang(text), is_mixed(text), round(dur, 3),
                              sha256_file(wav)))
    write_source(Source(
        set=name, title="CoSHE-Eval (code-switched Hindi-English, Soket Labs)",
        url=f"https://huggingface.co/datasets/{COSHE_REPO}", revision=revision, licence=licence,
        licence_evidence="dataset card metadata",
        terms="CC BY-NC 4.0: evaluation only; never committed, bundled or used for training",
        files=[f for _, f in fetched],
        notes="lang = en when the reference is entirely Latin script, otherwise hi (matrix language); "
        "mixed = reference has both Devanagari and Latin words",
    ), utts)
    log.info("%s: %d utterances", name, len(utts))


# -------------------------------------------------------------------------------------------- Kathbath

KATHBATH_REPO = "ai4bharat/Kathbath"
KATHBATH_LANGS = {"kathbath-hi": ("hindi", "hi"), "kathbath-ur": ("urdu", "ur")}


def prepare_kathbath(name: str) -> None:
    folder, lang = KATHBATH_LANGS[name]
    try:
        revision, licence = dataset_meta(KATHBATH_REPO)
        all_files = api.list_repo_files(KATHBATH_REPO, repo_type="dataset", revision=revision)
    except GatedRepoError:
        raise SystemExit(
            f"{KATHBATH_REPO} is gated: accept its terms at https://huggingface.co/datasets/{KATHBATH_REPO} "
            "and sign in with your own token (see eval/README.md), then run this again."
        ) from None
    files = sorted(f for f in all_files if f.endswith(".parquet") and f"/{folder}/" in f"/{f}"
                   and "test" in Path(f).name.lower() + str(Path(f).parent).lower()
                   and "known" not in f.lower())
    if not files:
        raise SystemExit(f"{KATHBATH_REPO}: no test parquet files for {folder}; layout changed, check the repo tree")
    fetched = [fetch(KATHBATH_REPO, revision, f) for f in files]
    first = pq.ParquetFile(fetched[0][0]).schema_arrow.names
    text_col = next(c for c in ("text", "transcription", "sentence") if c in first)
    id_col = next((c for c in ("fname", "id", "audio_file_name", "file_name") if c in first), None)
    heavy_step(f"resampling {name}")
    out = set_dir(name)
    utts: list[Utterance] = []
    cols = [c for c in (id_col, "audio", text_col) if c]
    for i, row in enumerate(iter_parquet_rows([p for p, _ in fetched], cols)):
        stem = Path(str(row.get(id_col) or row["audio"].get("path") or f"utt{i:06d}")).stem
        wav = out / "audio" / f"{stem}.wav"
        dur = convert_or_reuse(row["audio"]["bytes"], wav)
        text = row[text_col] or ""
        utts.append(Utterance(stem, rel_to(wav, out), text, lang, is_mixed(text), round(dur, 3), sha256_file(wav)))
    write_source(Source(
        set=name, title=f"Kathbath test, {folder} (AI4Bharat)", url=f"https://huggingface.co/datasets/{KATHBATH_REPO}",
        revision=revision, licence=licence,
        licence_evidence="dataset card metadata (the card text also says the packaging is CC0)",
        terms="Evaluation; gated download (terms accepted by the developer); not committed",
        files=[f for _, f in fetched], notes=f"test split files: {', '.join(files)}",
    ), utts)
    log.info("%s: %d utterances", name, len(utts))


def convert_or_reuse(data: bytes | None, wav: Path) -> float:
    """Resample to 16 kHz mono WAV, or reuse a WAV written by an earlier run of this script."""
    if wav.exists():
        import soundfile as sf

        return sf.info(wav).duration
    if data is None:
        raise ValueError(f"no audio for {wav.name}")
    return audio.convert_bytes(data, wav)


def coshe_lang(text: str) -> str:
    """CoSHE-Eval rows are Hindi-matrix code-switched speech, but some are entirely English."""
    return "en" if scripts_in(normalize(text)) == {LATIN} else "hi"


def rel_to(path: Path, base: Path) -> str:
    return path.relative_to(base).as_posix()


@dataclass
class SetSpec:
    prepare: Callable[[str], None]
    description: str


SETS: dict[str, SetSpec] = {
    **{k: SetSpec(prepare_fleurs, f"FLEURS test {v[0]} (CC BY 4.0)") for k, v in FLEURS_LANGS.items()},
    **{k: SetSpec(prepare_kathbath, f"Kathbath test {v[0]} (gated)") for k, v in KATHBATH_LANGS.items()},
    "coshe-eval": SetSpec(prepare_coshe, "CoSHE-Eval code-switched Hindi-English (CC BY-NC 4.0, eval only, ~9 GB)"),
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sets", nargs="*", help="sets to prepare (default: none; see --list)")
    parser.add_argument("--list", action="store_true", help="list the known sets")
    args = parser.parse_args()
    if args.list or not args.sets:
        for k, v in SETS.items():
            print(f"{k:12} {v.description}")
        return 0
    log_path = setup_logging("get_data")
    failed = []
    for name in args.sets:
        if name not in SETS:
            log.error("unknown set %s", name)
            failed.append(name)
            continue
        try:
            SETS[name].prepare(name)  # takes the busy lock (heavy_step) before resampling
        except SystemExit as e:
            log.error("%s: %s", name, e)
            failed.append(name)
        except Exception:  # keep going with the other sets; the log has the traceback
            log.exception("%s failed", name)
            failed.append(name)
        finally:
            release_heavy_slot()  # no-op unless this process holds the busy lock
    log.info("log: %s; failed: %s", rel(log_path), ", ".join(failed) or "none")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
