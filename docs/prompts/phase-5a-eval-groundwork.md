# Phase 5a: Evaluation groundwork (parallel track)

## Context
The test laptops arrive no earlier than 17 November 2026, so a second Claude Code session works in a git worktree beside the main track (`docs/LEGION-FIRST.md` sections 1 and 6). This phase pulls forward the parts of Phase 5 that don't depend on the post-processing queue: getting evaluation data, script-aware normalisers, model conversion and quantisation, and a harness that drives `wb-engine batch --file`. It starts after Phase 0 is merged and runs alongside Phases 1 to 3. Read `docs/PLAN.md` sections 5 and 6 and `docs/prompts/phase-5-languages-and-eval.md`.

## Goal
`eval/` and `scripts/models/` ready for Phase 5: download scripts for the public sets, tested normalisers, conversion and quantisation of every candidate fine-tune with checksums, and a harness that runs any model setup over any dataset and writes WER, CER and language-detection results, so Phase 5 only has to add the local sets, tune detection and decide.

## Steps
1. Work only in `eval/`, `scripts/models/` and `models/README.md`, on branch `phase-5a-eval-groundwork` in the worktree. Don't edit app, engine or design files; the main session owns them.
2. Data scripts (development time only, files under `eval/data/`, git-ignored): FLEURS test for en_us, hi_in and ur_pk (CC BY 4.0); Kathbath test for hi and ur; CoSHE-Eval (CC BY-NC 4.0, evaluation only, never committed or bundled). Each script records source URL, licence, file count and SHA-256 in `eval/data/SOURCES.md`, and resamples audio to 16 kHz mono WAV.
3. `eval/normalize.py` exactly as Phase 5 step 4 describes (English lower-case without punctuation; Hindi NFC, vowel signs kept, nukta forms unified; Urdu NFC with ي/ی, ك/ک, ه/ہ unified and optional diacritics removed; in mixed text each word normalised by its own script), with unit tests that include mixed Hindi–English sentences and a test proving vowel signs survive.
4. `scripts/models/convert_hf_whisper.py` around whisper.cpp's `models/convert-h5-to-ggml.py`, plus a quantise script for q8_0 and q5_0, for the candidates in `models/README.md`: `vasista22/whisper-hindi-small`, `vasista22/whisper-hindi-medium`, IndicWhisper Hindi and Urdu (confirm the licence first), `ARTPARK-IISc/whisper-medium-vaani-hindi` and `Trelis/whisper-hinglish-preview` (check its `<|mixedcode|>` token after conversion). Write each output's id, base, mel bins, licence, source and SHA-256 to `models/manifest.local.json` (git-ignored).
5. Harness `eval/run.py`: takes a model setup (one model, or per-language routing), a dataset and a role's settings; calls `wb-engine batch --file` (stubbed with a recorded JSONL fixture until Phase 2 is merged); writes per-utterance results and a summary with WER, CER, language-detection accuracy per chunk and how often English words came out in Devanagari. One command regenerates every table.
6. `eval/README.md`: how to download, convert, run and add the local sets in Phase 5.

## Verification
- `pytest eval` passes, including the normaliser tests.
- Data scripts finish on the Legion and `eval/data/SOURCES.md` lists every set with licence and checksum; nothing under `eval/data/` is tracked by git.
- Every candidate converts and quantises, or is recorded as dropped with the reason; a converted small model transcribes one FLEURS Hindi clip in Devanagari through whisper.cpp's own CLI.
- After Phase 2 is merged: `eval/run.py` runs whisper medium over 20 FLEURS clips per language through `wb-engine batch --file` and the numbers are within 3 WER points of the same model run through whisper.cpp's CLI.
- `asr-quality-auditor` (normalisation, licences, leakage) and `offline-auditor` (the app still never downloads; download code lives only in `eval/` and `scripts/`) report PASS.

## If Blocked
- A dataset needs a sign-in or licence acceptance: stop and tell the person exactly which page to accept, then continue with the rest.
- A checkpoint's licence is unclear or its download is gone: drop it and record why.
- Disk space runs short: keep q8_0 and q5_0 outputs, delete the intermediate f16 files, and report the totals.
