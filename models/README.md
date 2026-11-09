# Model pack

Model files are large and never committed. Phase 2 writes the download and quantisation scripts; Phase 5 converts the Indian-speech fine-tunes and decides which models ship, for which language and role. The app and engines only read files listed in `manifest.json`, check each SHA-256 at start-up and never download.

| Id (example) | Source | Base | Licence | Candidate role |
| --- | --- | --- | --- | --- |
| small | Hugging Face `ggerganov/whisper.cpp` (466 MiB) | small | MIT | English near-live on CPU |
| hindi-small-q8_0 | `vasista22/whisper-hindi-small`, converted and quantised | small | Apache 2.0 | Hindi near-live on CPU |
| hindi-medium-q8_0 | `vasista22/whisper-hindi-medium`, converted and quantised | medium | Apache 2.0 | Hindi rolling |
| indicwhisper-hi-q8_0 | AI4Bharat Vistaar (github.com/AI4Bharat/vistaar) | medium | MIT (confirmed 8 Nov 2026, see below) | Hindi rolling |
| vaani-hi-q8_0 | `ARTPARK-IISc/whisper-medium-vaani-hindi` | medium | Apache 2.0 | Hindi rolling |
| hinglish-q5_0 | `Trelis/whisper-hinglish-preview`, converted and quantised | large-v3 | Apache 2.0 | Code-switched English–Hindi night pass; rolling where a GPU makes it fast enough; GPU live |
| indicwhisper-ur-q8_0 | AI4Bharat Vistaar | medium | MIT (confirmed 8 Nov 2026, see below) | Urdu rolling |
| large-v3-turbo-q5_0 | Hugging Face `ggerganov/whisper.cpp` (547 MiB) | large-v3-turbo | MIT | English rolling on CPU; GPU live |
| large-v3-q5_0 | same (1.1 GiB) | large-v3 | MIT | Night pass on CPU |
| large-v3 | same (2.9 GiB) | large-v3 | MIT | GPU live; translation |
| silero-v6.2.0 | whisper.cpp `models/download-vad-model.sh silero-v6.2.0` | Silero VAD | MIT | Voice activity detection |

Code-switching: in English + Hindi sessions each sentence-sized chunk is detected and either routed to that language's model (both loaded) or sent to one code-switching model; Phase 5 decides which (`defaults.routing`).

Roles: **near-live** (greedy, one utterance at a time, CPU tier C+), **rolling** (beam 5 while recording continues), **night** (beam 5 overnight, most accurate that fits), **live** (GPU streaming), **translate** (large-v3 only; turbo cannot translate).

## Converting and quantising

Scripts in `scripts/models/` do this on the Legion at development time (details and commands in `eval/README.md`):

1. `build_whispercpp.py` builds whisper.cpp v1.9.5 (`whisper-cli`, `whisper-quantize`) portably into `eval/.tools/`.
2. `convert_hf_whisper.py` downloads each candidate in `scripts/models/candidates.toml` at a pinned revision, checks its licence, mel bins (80 for small and medium, 128 for large-v3) and special-token ids, and converts it with whisper.cpp's `models/convert-h5-to-ggml.py` to `models/converted/ggml-<id>-f16.bin`.
3. `quantize_models.py` makes `q8_0` and `q5_0` with `whisper-quantize`. Phase 5 ships `q5_0` only when its WER stays within 1 point of `q8_0` for that language.
4. Every output is recorded in `models/manifest.local.json` (git-ignored) with id, file, SHA-256, bytes, base, quantisation, mel bins, languages, candidate roles, licence, licence evidence, source revision and whisper.cpp version; dropped candidates are listed with the reason. Phase 5 copies validated entries into `manifest.json` (see `manifest.example.json`).
5. `download_ggml.py` fetches stock models (small, medium, large-v3 and its q5_0, turbo q5_0, Silero VAD) from `ggerganov/whisper.cpp` and `ggml-org/whisper-vad` at a pinned revision.

### Licence confirmation: IndicWhisper (AI4Bharat Vistaar)

Confirmed MIT for the Hindi and Urdu checkpoints. Vistaar README, section License: "Vistaar is MIT-licensed. The license applies to all the fine-tuned language models" ([README at commit 21dce34](https://github.com/AI4Bharat/vistaar/blob/21dce3406fddf0e18984165e735d23db38760dd3/README.md#license), read 8 November 2026). The repository's LICENSE file is the MIT License, "Copyright (c) 2023 AI4Bhārat". Checkpoints: `https://indicwhisper.objectstore.e2enetworks.net/hindi_models.zip` and `urdu_models.zip` (SHA-256 of each archive in `models/converted/<id>.convert.json`).

## Packs

- **CPU pack:** the models validated for near-live and rolling roles, plus the night model if Phase 5 found one worth running on a CPU.
- **GPU pack:** the CPU pack plus large-v3 for live captions and translation, and the code-switching model if validated.
