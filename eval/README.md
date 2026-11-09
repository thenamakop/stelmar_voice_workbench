# Evaluation (`eval/`) and model conversion (`scripts/models/`)

Development-time tools for choosing a model per language and role (docs/PLAN.md sections 5 and 6).
Nothing here is bundled with the app, and the app never runs it: downloads happen only in
`eval/` and `scripts/`, never at runtime. Phase 5a built this groundwork; Phase 5 adds the local
STELMAR sets, tunes language detection and decides.

| Path | What |
| --- | --- |
| `get_data.py` | Downloads the public sets and prepares 16 kHz mono WAV with a manifest per set |
| `normalize.py` | Script-aware normaliser used for every score (tests in `tests/test_normalize.py`) |
| `metrics.py` | WER, CER, English words written in Devanagari, wrong-script output, chunk language detection |
| `run.py` | Harness: model setup × dataset × role → per-utterance results, summary and tables |
| `roles.toml`, `runs.toml` | Decoding settings per role; every run and comparison that `run.py all` regenerates |
| `wbeval/` | Shared helpers: paths, gate lock, priority, audio, manifests, engines, tool versions |
| `../scripts/models/` | whisper.cpp build, checkpoint conversion, quantisation, stock model download |

Git-ignored (see `eval/.gitignore` and the root `.gitignore`): `eval/.venv/`, `eval/.tools/`,
`eval/data/`, `eval/results/`, `eval/logs/`, `models/*.bin`, `models/converted/`,
`models/manifest.local.json`. CoSHE-Eval (CC BY-NC 4.0) is for evaluation only and is never
committed, bundled or used for training.

## Sharing the Legion with Track A

Track A runs timing-sensitive gates (soak, load and kill tests under `emu-low`) on the same
machine. Heavy steps here (whisper-cli and harness decoding, conversion, quantisation, building
whisper.cpp) go through `wbeval.common.run_heavy`, strictly one at a time:

1. **Gate request and gate lock:** while `D:\Projects\stelmar-gate-request.flag` (Track A wants the machine
   soon) or `D:\Projects\stelmar-gate.lock` (a gate is running) exists, wait and log it every 2 minutes; no
   step takes the busy lock while the request flag is up. Dataset resampling takes the busy lock like
   any other heavy step; pytest tests marked `heavy` wait for both files.
2. **Memory:** start only when at least 6 GB of RAM is free (8 GB for large-v3-class models such as the
   Hinglish fine-tune); otherwise wait and re-check every 2 minutes.
3. **Busy lock (the other direction):** while the step runs, hold `D:\Projects\stelmar-eval-busy.lock`
   (step name, PID, start time), created after the gate lock is free and deleted in a `finally` block,
   so Track A can wait for it. It is never held while waiting, so the two sessions cannot deadlock; a lock
   left by a process that no longer exists is replaced.
4. **Priority:** below normal.

Every wait (gate lock, memory, busy lock) is logged with a timestamp, kept per utterance in
`utterances.jsonl` and listed under `waits` in the run's `summary.json`; RTF leaves the waits out.
Downloads, editing and the plain unit tests do not wait: `eval/conftest.py` points every lock and flag
at a temporary folder for each unit test, and every test that fakes `time.sleep` fails after a few
calls, so a unit test can never wait or spin on Track A's real files. Override the lock paths with
`WB_GATE_REQUEST`, `WB_GATE_LOCK` and `WB_BUSY_LOCK` only for testing.

## Setup (Legion, PowerShell, from the repository folder)

```powershell
uv sync --project eval                                       # Python 3.13 env in eval\.venv (CPU-only torch)
eval\.venv\Scripts\python scripts\models\build_whispercpp.py # clones and builds whisper-cli, whisper-quantize
```

`HF_HOME` must point at `D:\caches\huggingface` (docs/SETUP.md) so downloads stay off C:.

### whisper.cpp version

| Tool | Version | Commit | Build |
| --- | --- | --- | --- |
| whisper.cpp (`whisper-cli`, `whisper-quantize`, `models/convert-h5-to-ggml.py`) | v1.9.5 | `d1be6fde11ac6e0407606b4e42fe72d34add8037` | CPU only, `GGML_NATIVE=OFF`, AVX, AVX2, FMA, F16C (the `avx2` engine's flags) |
| openai/whisper (mel filter assets for conversion) | v20250625 | `31243bad24cc746f07d4c8bfdd2d974872cb1803` | |

The pin lives in `wbeval/tools.py`; `eval/.tools/whisper.cpp.json` records what was built, and every
results summary copies it. **Phase 2 comparison:** `wb-engine` bundles whisper.cpp through
whisper-rs-sys. The "within 3 WER points of whisper-cli" check is only meaningful with the same
whisper.cpp version on both sides: rebuild whisper-cli at wb-engine's version (change
`WHISPER_CPP_TAG`, delete `eval/.tools/whisper.cpp`, run the build script), or record the difference
next to the result. `eval/results/TABLES.md` prints both versions in the comparison table.

**What Phase 2's `wb-engine batch --file` must match for that comparison:**

- **Chunks of at most 15 s.** With timestamps off, decode VAD chunks of at most `max_chunk_s`, each on
  its own, never a whole 30 s window (see the GATES open item), and flag a chunk that reaches the token limit.

- **No model timestamps.** Every role decodes with `timestamps = false` (whisper-cli `-nt`; whisper-rs
  `FullParams::set_no_timestamps(true)`). Segment times come from the audio clock and VAD (PLAN section 5).
  The Hindi fine-tunes break with timestamp decoding (`docs/evidence/phase-5a/timestamps-decoding.txt`).
- **Role settings.** The harness passes them as `--settings <json>` (the fields of `Settings` in
  `wbeval/engines.py`), on top of the `--file`, `--models`, `--langs` and `--out` the Phase 2 prompt
  names. Phase 2 can accept that flag or change the adapter in `WbEngine.command`.

**Known gaps between the whisper-cli path and the app** (recorded, not hidden):

- whisper-cli detects the language once per clip (first 30 s) among all languages; the app detects per
  sentence-sized chunk among the session's languages. When whisper-cli detects a language outside the
  session's set, the harness takes the first non-English expected language, never English (PLAN section 5
  rule 3), and counts it as `detected_outside_set`.
- Near-live `audio_ctx` sized to the utterance (PLAN section 6) is not passed to whisper-cli.
- whisper-cli is driven like the app: each clip is cut at VAD pauses (whisper.cpp's own Silero VAD,
  `whisper-vad-speech-segments`) into chunks of at most `max_chunk_s` (15 s for every role here), and each
  chunk is decoded on its own without model timestamps. Without chunking, whisper.cpp decodes 30 s
  windows, stops at the token limit and skips the rest (Phase 5a found 20 to 41% of words lost on CoSHE-Eval);
  every summary reports `truncated_utterances` and `longest_chunk_s` to catch this.
- Rolling and night carry the last 200 characters of the previous chunk as the prompt (after the
  glossary), one whisper-cli call per chunk; near-live decodes all chunks of a clip in one call with no
  context. The app carries context only within the same language; whisper-cli runs are single-language.
- Rolling and night use 15-second chunks (the multi-language session size) on single-language sets too,
  where the app may use up to 30 seconds. This made FLEURS Hindi worse than whole-clip decoding (stock
  medium 47.5 to 51.7 WER, hindi-medium 18.5 to 23.9) because some cuts fall without a pause; Phase 5 adds a
  30-second variant for single-language sets before ranking anything on them.
- The previous chunk's text is passed on as context and is not reset after a flagged chunk (a repetition
  loop can poison the next prompt), unlike the app (PLAN section 6). Phase 5 adds the reset or a context-off
  control run before comparing with stock medium.
- **RTF is not comparable across roles or with wb-engine:** whisper-cli wall time per clip. Rolling, night and live run one whisper-cli process per chunk, so the model is reloaded for every chunk; near-live runs one process per clip. RTFs of different roles are therefore not comparable with each other or with wb-engine, which keeps the model loaded; speed decisions use `wb-engine bench` (Phase 2), not this column.

## Data

```powershell
eval\.venv\Scripts\python eval\get_data.py --list
eval\.venv\Scripts\python eval\get_data.py fleurs-en fleurs-hi fleurs-ur coshe-eval
eval\.venv\Scripts\python eval\get_data.py kathbath-hi kathbath-ur          # gated, see below
```

| Set | Source | Licence | Size | Notes |
| --- | --- | --- | --- | --- |
| `fleurs-en`, `fleurs-hi`, `fleurs-ur` | `google/fleurs` test (en_us, hi_in, ur_pk) | CC BY 4.0 | about 1.5 GB | Reference: `raw_transcription`. vasista22 models trained on FLEURS train and dev: their FLEURS numbers are not independent |
| `kathbath-hi`, `kathbath-ur` | `ai4bharat/Kathbath` test | CC BY 4.0 (card text also says CC0) | a few GB | Gated: accept the terms first |
| `coshe-eval` | `soketlabs/CoSHE-Eval` | CC BY-NC 4.0 | about 9 GB, 1,985 utterances, about 30 h | Code-switched Hindi–English, Hindi in Devanagari and English in Latin script; evaluation only |

Each set gets `eval/data/<set>/manifest.jsonl` (id, audio, reference text, language, mixed, duration,
WAV SHA-256) and `source.json`; `eval/data/SOURCES.md` lists every set with its source URL, pinned
repository revision, licence, file count and the SHA-256 of every downloaded file and of the manifest.

### Kathbath sign-in (gated dataset)

1. Signed in to Hugging Face, accept the terms at https://huggingface.co/datasets/ai4bharat/Kathbath.
2. Create a **read** token at https://huggingface.co/settings/tokens.
3. In your own PowerShell window (not inside an agent session), from the repository folder, run:

   ```powershell
   eval\.venv\Scripts\hf.exe auth login
   ```

   Paste the token at the prompt and answer **n** to "Add token as git credential?". With
   `HF_HOME=D:\caches\huggingface` the token is written to `D:\caches\huggingface\token` (and
   `D:\caches\huggingface\stored_tokens`), outside the repository. Never paste it into a file in the
   repository, into `docs/evidence/`, into a log or into a chat.
4. Run `get_data.py kathbath-hi kathbath-ur`. The scripts read the token through `huggingface_hub`
   and never print or log it.

## Models

```powershell
eval\.venv\Scripts\python scripts\models\download_ggml.py small medium silero-v6.2.0   # stock baselines + VAD
eval\.venv\Scripts\python scripts\models\convert_hf_whisper.py --download-only          # fetch all candidates
eval\.venv\Scripts\python scripts\models\convert_hf_whisper.py                          # convert to f16 ggml
eval\.venv\Scripts\python scripts\models\quantize_models.py                             # q8_0 and q5_0
eval\.venv\Scripts\python scripts\models\quantize_models.py --delete-f16                # if disk runs short
```

Candidates, pinned revisions and licence evidence are in `scripts/models/candidates.toml`. The
converter refuses anything not MIT, Apache 2.0, BSD or ISC; checks mel bins (80 for small and
medium, 128 for large-v3) and encoder depth; and compares every special-token id whisper.cpp will
derive from the vocabulary size with the checkpoint's own tokenizer (an added token such as
`<|mixedcode|>` shifts them). Results per candidate: `models/converted/<id>.convert.json`; every
output with SHA-256 in `models/manifest.local.json`, including dropped candidates and why.

## Running

```powershell
$env:WB_MACHINE = "Legion"   # machine name recorded in every summary
eval\.venv\Scripts\python eval\run.py run --setup medium --dataset fleurs-hi --role rolling --limit 20
eval\.venv\Scripts\python eval\run.py run --setup "en=medium,hi=indicwhisper-hi-q8_0" --dataset coshe-eval `
    --role rolling --lang-mode detect --langs en,hi --limit 50
eval\.venv\Scripts\python eval\run.py all       # every run in runs.toml, then eval\results\TABLES.md
eval\.venv\Scripts\python eval\run.py tables    # tables only
eval\.venv\Scripts\python eval\run.py rescore   # recompute scores from finished runs (no decoding)
```

- **Setups:** a model id (`medium`), a named setup from `runs.toml`, or per-language routing
  (`en=medium,hi=hindi-medium-q8_0`).
- **Roles:** `near-live`, `rolling`, `night`, `live` from `roles.toml` (beam, fallback, context, VAD,
  chunk length, threads).
- **Engines:** `whisper-cli` (reference) and `wb-engine` (`batch --file`, the app's engine; set
  `WB_ENGINE` to the built binary). Until Phase 2 is merged, `wb-engine` runs in replay mode with
  `--engine-fixture <folder>`: one JSONL file of engine events per utterance, recorded from whisper-cli
  with `run.py record-fixture` into a folder you keep out of git. The committed
  `tests/fixtures/wb-engine/synthetic-hi/` is hand-written and only exercises the event parser.
- **Outputs:** `eval/results/<run>/utterances.jsonl` (resumable) and `summary.json`: WER; CER for
  Hindi and Urdu; share of the reference's English words that came out in Devanagari; script mix and
  wrong-script rate (catches translation); per-chunk language detection with its confusion matrix;
  RTF; model SHA-256; dataset manifest SHA-256; whisper.cpp version; machine and emulation profile.
- **Language detection** is measured only in `--lang-mode detect`, against the utterance's language from
  the manifest. In `fixed` mode the engine is told the language, so the tables print `n/a`. For
  CoSHE-Eval, `fixed` is an oracle: the language comes from the reference (`en` when the reference is
  entirely Latin script, otherwise `hi`, the matrix language). English chunks inside a Hindi utterance
  count as misses until Phase 5's local code-switched set adds chunk-level labels.
- **Independence:** `scripts/models/candidates.toml` lists, per candidate, the sets whose corpus it was
  trained on (`not_independent_on`) and the sets whose sources do not say (`independence_unknown_on`),
  with the evidence. A summary gets `independent: false` (the tables print **no**) or `independent: null`
  (**unknown**); neither is ranked, and such numbers only show that a conversion works. All Hindi
  fine-tunes here saw FLEURS Hindi in training, IndicWhisper also saw Kathbath, and for the Hinglish model
  CoSHE-Eval is unknown: its card names no training data, only its starting checkpoint, and mentions CoSHE
  only as an evaluation benchmark.
- **No dataset text is committed:** FLEURS and CoSHE-Eval references, and model output for their clips
  beyond a short failure marker, stay under `eval/data/` and `eval/results/` (git-ignored). Evidence files
  describe outputs instead of quoting them; committed fixtures are hand-written.
- **Latin→Devanagari rate** counts every Latin-script word in the reference as English. CoSHE-Eval
  references also contain a little romanised Hindi (for example "hai", "ki", "toh"), so the rate slightly
  overstates English words.
- **Stale results:** each run stores a fingerprint of its settings, model SHA-256s, dataset manifest and
  whisper.cpp commit. `run.py all` re-runs any run whose fingerprint changed, and `run.py run` refuses to
  resume one without `--force`.

## Normalisation (`normalize.py`)

Applied identically to references and hypotheses, word by word according to each word's script:

- **English (Latin):** lower-case, punctuation removed (apostrophes join, hyphens split).
- **Hindi (Devanagari):** NFC; vowel signs, virama, anusvara, chandrabindu and nukta kept; nukta
  letters unified (precomposed क़ ऩ ऱ ऴ and decomposed forms compare equal); danda and ZWJ/ZWNJ removed.
- **Nukta, two scores** (PLAN section 6, "nukta forms unified"; decided 8 Nov 2026):
  - **Primary** WER and CER, used by every choice rule: the nukta is kept and only the two Unicode
    encodings of the same letter are unified, so क़ = क + ़ but ज़ ≠ ज.
  - **Secondary**, "WER (nukta ignored)": U+093C removed before scoring (`normalize.drop_nukta`), so
    ज़ = ज, because transcribers use the nukta inconsistently.
  - The tables' "Nukta rank check" flags every dataset where the two scores rank two runs differently,
    for Phase 5 to look at.
- **Urdu (Perso-Arabic):** NFC; ي/ى → ی, ك → ک, ه → ہ; ی + hamza → ئ; optional diacritics (harakat,
  shadda, sukun, superscript alef, Quranic marks) and tatweel removed; ھ, hamza and madda kept.
- **All:** invisible format characters (LRM, RLM, ALM, BOM, ZWSP, soft hyphen) removed; digits in any
  script become ASCII; thousands separators dropped; a token that changes
  script mid-word is split.

Whisper's default `BasicTextNormalizer` is never used: it deletes combining marks and so strips Hindi
vowel signs (`tests/test_normalize.py::test_whisper_basic_normaliser_would_have_stripped_them`).

## Tests

```powershell
eval\.venv\Scripts\python -m pytest eval            # unit tests; tests marked heavy wait for the gate lock
```

## Adding the local STELMAR sets (Phase 5)

The local sets are recorded after Phase 4 (docs/TEST-AUDIO.md) and live in `eval/local/` (git-ignored,
private): one `.wav` and one `.txt` with the same name per clip, Hindi in Devanagari and English in
Latin script.

1. Add a `prepare_local` function to `get_data.py` that reads `eval/local/<set>/*.wav` and the matching
   `.txt`, resamples with `wbeval.audio.convert_bytes`, sets `lang` (`en`, `hi`, `ur`; `hi` for the mixed
   set) and `mixed`, and calls `write_source` with licence "private, consented recordings; never
   committed". Register `local-en`, `local-hi`, `local-ur` and `local-mixed` in `SETS`.
2. For chunk-level language labels on the mixed set, add a `chunks` field to the manifest (start, end,
   language) and score detection against it in `run.summarise`.
3. Add the candidate grid (every candidate and routing setup × every set × its roles, q8_0 and q5_0)
   to `runs.toml`, and the Phase 2 wb-engine runs without `needs`.
4. `run.py all` regenerates every table; Phase 5 writes `eval/REPORT.md` from them.
