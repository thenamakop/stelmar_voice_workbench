# Phase 5: Languages, code-switching, evaluation and model choice

## Context
Post-processing works end to end. This phase makes English–Hindi code-switching, Hindi and Urdu first-class and picks the model for each language and role with evidence (decisions D7, D8 and D14). Speakers switch between English and Hindi at any point, and the room machines are 16 GB laptops, so accuracy on mixed speech and speed on the low-baseline laptop both decide. Read `docs/PLAN.md` sections 5 and 6.

## Goal
Correct rendering for Devanagari, right-to-left Nastaliq Urdu and mixed Hindi–English text; tuned per-chunk language detection; converted and quantised Indian-speech and code-switching fine-tunes; and an evaluation harness whose `eval/REPORT.md` ranks every candidate and routing setup per language and role and sets the validated roles and defaults in `models/manifest.json`.

## Steps
1. Rendering: `lang` and `dir` on every segment element; Urdu right-aligned with `--font-urdu` and `--lh-urdu`; Hindi with `--font-devanagari` and `--lh-devanagari`; mixed segments use `dir="auto"` and render each word in its own script and font. Check selection, copy and caret movement for Urdu and mixed text in an editable field.
2. Conversion script `scripts/models/convert_hf_whisper.py` around whisper.cpp's `models/convert-h5-to-ggml.py` for `vasista22/whisper-hindi-small` and `vasista22/whisper-hindi-medium` (Apache 2.0), IndicWhisper Hindi and Urdu (AI4Bharat Vistaar; confirm the licence), `ARTPARK-IISc/whisper-medium-vaani-hindi` (Apache 2.0) and `Trelis/whisper-hinglish-preview` (Apache 2.0; check that its `<|mixedcode|>` token survives conversion). Quantise each to q8_0 and q5_0 with whisper.cpp's `quantize`. Record source, licence and SHA-256 in `models/manifest.json`.
3. Evaluation data in `eval/`: FLEURS test (en_us, hi_in, ur_pk), Kathbath test (hi, ur), CoSHE-Eval (code-switched Hindi–English; CC BY-NC 4.0, evaluation only, never committed), and the local STELMAR sets in `eval/local/` (15 minutes each of English, Hindi and Urdu, and 30 minutes of natural code-switched English–Hindi talk with Hindi in Devanagari and English in Latin script; private, so always git-ignored). Every candidate runs through `wb-engine batch --file` with the exact settings of its role, not through a separate Python model.
4. Metrics with `jiwer`: WER for all languages; CER for Hindi and Urdu; language-detection accuracy per chunk on the code-switched sets; using `eval/normalize.py` (English lower-case without punctuation; Hindi NFC, vowel signs kept, nukta forms unified; Urdu NFC with ي/ی, ك/ک, ه/ہ unified and optional diacritics removed; in mixed segments each word normalised by its script). Report separately how often English words came out in Devanagari. Unit tests for the normalisers.
5. Language detection tuning on the code-switched sets: chunk length (3 to 15 seconds), the 0.8 neighbour rule for short chunks, the 0.2 both-ways margin and the 0.5 no-forced-English rule. Report the confusion matrix and how many chunks were decoded both ways (the cost).
6. Routing or one model: compare (a) per-chunk routing to per-language models (English model for English chunks, Hindi model for Hindi chunks, both loaded) with (b) a single code-switching or multilingual model, on the local code-switched set and the single-language sets, on each laptop.
7. Speed: each candidate's and setup's throughput, RTF and peak memory from `wb-engine bench` on the low-baseline laptop and the mid laptop (each device configuration) and on the Legion (GPU).
8. Decide per language and role with the PLAN section 6 rules: rolling = lowest local WER (code-switched set weighted equally with the single-language sets) with RTF ≤ 1.0 in the low-baseline laptop's best configuration; night = lowest local WER overall; near-live = local WER no more than 1.5 times the rolling WER and RTF ≤ 0.35; q5_0 only if within 1 WER point of q8_0; ties go to the smaller model. Apply D14: Latin script for English words inside Hindi sentences when a model that does this wins; otherwise record the fallback. Write `eval/REPORT.md` (tables per language, dataset, role and routing setup; detection accuracy; speed with machine and configuration names; sources; licences) and set `validated_roles` and `defaults` in `models/manifest.json`.

## Verification
- `eval/REPORT.md` is committed with numbers for every candidate, setup, dataset and role, regenerable with one command.
- The chosen setup beats zero-shot Whisper medium on every local set, including the code-switched set, and detects the chunk language correctly at least 90% of the time there.
- No chunk in the local code-switched set comes back as an English translation of Hindi speech.
- Re-running the system check picks up the new defaults on both laptops (attach the `capability.json` diffs).
- Screenshots: Hindi, Urdu and mixed segments render correctly on Record and Review (no missing-glyph boxes; Urdu right-aligned; English words in mixed segments in Latin script where the model produced them).
- `asr-quality-auditor` (normalisation, leakage, reproducibility; the vasista22 models were trained on FLEURS train and dev, so their FLEURS numbers are marked not independent; CoSHE-Eval used for evaluation only), `design-consistency-reviewer` and `offline-auditor` report PASS.

## If Blocked
- The local sets are not ready yet (they are recorded after Phase 4; `docs/TEST-AUDIO.md`): run every step on the public sets first, mark the local-set Verification items DEFERRED in `docs/GATES.md`, and finish them when the sets arrive. Manifest defaults stay provisional until then.
- A checkpoint's licence is unclear or its download is gone: drop it and record why; plain Whisper stays the fallback.
- A converted model outputs garbage: check mel bins (large-v3 and its fine-tunes use 128, small and medium 80) and the tokenizer files, then convert again.
- The Hinglish model's special token is lost in conversion: keep it for evaluation through `--file` only if its output is still correct, otherwise drop it and note D14's fallback.
- No candidate meets the near-live bar for a language: near-live stays off for that language (expected for Urdu); record it in the report.
- If decision D1 meant SeamlessM4T: evaluate it in Python only, report it, and ask before adding a second runtime.
