# Model pack

Model files are large and never committed. Phase 2 writes the download and quantisation scripts; Phase 5 converts the Indian-speech fine-tunes and decides which models ship, for which language and role. The app and engines only read files listed in `manifest.json`, check each SHA-256 at start-up and never download.

| Id (example) | Source | Base | Licence | Candidate role |
| --- | --- | --- | --- | --- |
| small | Hugging Face `ggerganov/whisper.cpp` (466 MiB) | small | MIT | English near-live on CPU |
| hindi-small-q8_0 | `vasista22/whisper-hindi-small`, converted and quantised | small | Apache 2.0 | Hindi near-live on CPU |
| hindi-medium-q8_0 | `vasista22/whisper-hindi-medium`, converted and quantised | medium | Apache 2.0 | Hindi rolling |
| indicwhisper-hi-q8_0 | AI4Bharat Vistaar (github.com/AI4Bharat/vistaar) | medium | MIT (confirm in Phase 5) | Hindi rolling |
| vaani-hi-q8_0 | `ARTPARK-IISc/whisper-medium-vaani-hindi` | medium | Apache 2.0 | Hindi rolling |
| hinglish-q5_0 | `Trelis/whisper-hinglish-preview`, converted and quantised | large-v3 | Apache 2.0 | Code-switched English–Hindi night pass; rolling where a GPU makes it fast enough; GPU live |
| indicwhisper-ur-q8_0 | AI4Bharat Vistaar | medium | MIT (confirm in Phase 5) | Urdu rolling |
| large-v3-turbo-q5_0 | Hugging Face `ggerganov/whisper.cpp` (547 MiB) | large-v3-turbo | MIT | English rolling on CPU; GPU live |
| large-v3-q5_0 | same (1.1 GiB) | large-v3 | MIT | Night pass on CPU |
| large-v3 | same (2.9 GiB) | large-v3 | MIT | GPU live; translation |
| silero-v6.2.0 | whisper.cpp `models/download-vad-model.sh silero-v6.2.0` | Silero VAD | MIT | Voice activity detection |

Code-switching: in English + Hindi sessions each sentence-sized chunk is detected and either routed to that language's model (both loaded) or sent to one code-switching model; Phase 5 decides which (`defaults.routing`).

Roles: **near-live** (greedy, one utterance at a time, CPU tier C+), **rolling** (beam 5 while recording continues), **night** (beam 5 overnight, most accurate that fits), **live** (GPU streaming), **translate** (large-v3 only; turbo cannot translate).

## Converting and quantising

1. Convert a Hugging Face Whisper checkpoint with whisper.cpp's `models/convert-h5-to-ggml.py` (it needs clones of `openai/whisper` and `whisper.cpp` beside the checkpoint). small and medium use 80 mel bins; large-v3 uses 128.
2. Quantise with whisper.cpp's `quantize` tool to `q8_0` and `q5_0`. Phase 5 ships `q5_0` only when its WER stays within 1 point of `q8_0` for that language.
3. Record id, file, SHA-256, bytes, base, quantisation, languages, candidate and validated roles, licence and source in `manifest.json` (see `manifest.example.json`).

## Packs

- **CPU pack:** the models validated for near-live and rolling roles, plus the night model if Phase 5 found one worth running on a CPU.
- **GPU pack:** the CPU pack plus large-v3 for live captions and translation, and the code-switching model if validated.
