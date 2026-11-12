# STELMAR Voice Workbench: Build Plan (version 2, CPU-first)

As of 7 November 2026. Replaces version 1, which assumed a GPU laptop at the venue.

Build a desktop app that records every STELMAR 2027 session and produces transcripts on the venue computer itself, with no network. English and Hindi are the primary languages and a speaker may switch between them at any point, even mid-sentence; Urdu is also supported. The venue computers are not decided yet and will differ from room to room, so the app is built for the whole range of ordinary 16 GB laptops: it records each talk and transcribes it in the background, one closed 10-minute part at a time, using the CPU and, where it helps, the integrated or discrete GPU together. Live text is off by default everywhere; before each recording the operator chooses the mode, and live text is offered only on machines the first-launch system check has cleared. Claude Code builds it in nine gated phases (`docs/prompts/phase-0-*.md` to `phase-8-*.md`).

## What changed from version 1

| Area | Version 1 | Version 2 |
| --- | --- | --- |
| Venue machines | One laptop with an RTX 4070 | Undecided 16 GB laptops that differ by room; the Lenovo Legion Pro 5 is the build machine and can go to any room |
| Default mode | Live captions, accurate pass afterwards | Record + post-process on every machine; live text off by default and chosen per recording |
| Languages | One language per session, switched by hand | English and Hindi primary, detected sentence by sentence; code-switching inside a sentence handled; Urdu supported |
| First launch | Nothing | System check: hardware probe, speed test per model and device, verdict saved as a capability profile |
| Post-processing devices | GPU | CPU, integrated or discrete GPU, or both together, whichever the check measures fastest |
| Engine | whisper.cpp inside the app process | whisper.cpp in separate engine processes, one build per backend, post-processing at low priority |
| Build order | Live pipeline in Phase 3 | Recording, engines, system check and post-processing first; live mode in Phase 7 |

## 1. Decisions

Each decision has a default that the prompts already assume. Changing one only reworks the part named in the last column.

| # | Decision | Default | Alternative | Rework if changed |
| --- | --- | --- | --- | --- |
| D1 | What "Whisper Large and C" means | Whisper weights running on whisper.cpp, the C/C++ Whisper runtime | Meta's SeamlessM4T: PyTorch only, weights CC BY-NC 4.0; evaluation-only Urdu challenger | One extra evaluation run in Phase 5 |
| D2 | Venue machines | Undecided and different per room: plan for any 16 GB x86-64 laptop with AVX2, from 4-core low-power models with basic integrated graphics upwards. Each room's machine runs its own system check | One known model everywhere | Tier thresholds only |
| D3 | Where Whisper runs | Separate `wb-engine` processes built per backend (CPU AVX2, CPU AVX-512, Vulkan, CUDA; OpenVINO experimental); the app itself never links whisper.cpp | Engine inside the app process | Phases 2 to 4 |
| D4 | Mode per recording | Record + post-process is the default on every machine, GPU or not. Before each recording, the Start Session dialog asks for the mode; Live + post-process is offered only where the profile clears it for all of the session's languages | Live by default on GPU machines | Setting only |
| D5 | Live text on CPU-only machines | Never on by default; offered in the dialog only when the small models pass the speed bar on that machine and the Phase 5 accuracy bar (expected: English and Hindi, not Urdu) | Never offered on CPU | Phase 7 |
| D6 | Post-processing passes (confirmed) | Rolling pass during the session (each closed part); night pass with a larger model when it is more accurate and fits before 08:00 | One pass after each session | Phase 4 scheduler |
| D7 | Model per language and pass | Chosen by Phase 5 on local recordings, including code-switched speech, from the candidates in section 5 | large-v3 everywhere (too slow on CPU) | None: Phase 5 decides |
| D8 | Languages in a session | English and Hindi primary. Each session lists its expected languages (default English + Hindi); the language is detected sentence by sentence among them. The operator can lock one language (Alt+1/2/3) and return to detection (Alt+4) | One fixed language per session | Phases 4, 5 and 7 |
| D9 | English text for Hindi and Urdu speech | Live on GPU machines only (large-v3 translate task); optional low-priority translation job after post-processing | Off | Setting only |
| D10 | Backup audio | 48 kHz 16-bit WAV in 10-minute parts, header flushed every second, mirrored to a second drive | FLAC (about half the size) | Recorder |
| D11 | Speaker names | Tagged by the operator; no automatic diarisation | pyannote | New phase |
| D12 | Transcript deadline (confirmed) | Rolling transcript ready within about 20 minutes of a session's end when the machine keeps up; every transcript final by 08:00 the next day | No deadline | Scheduler setting |
| D13 | Devices for post-processing | Measured per machine: CPU only, GPU only (integrated or discrete), or both at once (two engines on separate work units, or the OpenVINO encoder on an Intel GPU with decoding on the CPU). The fastest configuration that leaves the recorder untouched wins | CPU only | Phases 3 and 4 |
| D14 | English words inside Hindi sentences | Written in Latin script as spoken, when a code-switching model passes Phase 5; otherwise as the chosen Hindi model writes them | Always Devanagari | Phase 5 choice |

## 2. Machines and tiers

The venue PCs are not decided and will differ from room to room, so nothing assumes a particular machine. The app plans for the whole range of ordinary 16 GB laptops and lets each machine's system check decide what it does.

| Class | Typical machine | Integrated or discrete GPU | Expected role |
| --- | --- | --- | --- |
| Low baseline | 4 cores, 8 threads, 15 W (older Core i3/i5 U-series, Ryzen 3/5 U-series) | Intel UHD or Iris Xe, AMD Radeon Vega | Record + post-process; hybrid only if the check shows a gain |
| Mid | 6 to 10 cores (Core i5/i7 H or P-series, Ryzen 5/7 H-series) | Iris Xe, Intel Arc (Core Ultra), Radeon 680M/780M | Record + post-process with CPU + GPU; near-live text possible |
| GPU laptop | Any of the above with an NVIDIA RTX or similar | Discrete, 6 GB+ VRAM | Everything, including live captions (the Legion is one) |

| Tier | Live text offered in the Start Session dialog | Post-processing | Gate measured by the system check |
| --- | --- | --- | --- |
| G: GPU live | Streaming captions in English, Hindi and Urdu | Rolling and night passes on the best device configuration | A GPU engine works and the live model runs at RTF ≤ 0.5 in streaming settings |
| C+: near-live | Near-live text a few seconds behind, for languages with a validated small model | Rolling pass with a medium-class model; night pass if it fits | Small model RTF ≤ 0.35 on half the threads, and a rolling configuration at RTF ≤ 1.0 |
| C: record + post-process | None | Rolling pass with the most accurate model that keeps up; night pass if it fits | At least one validated model meets the deadline |
| R: record only | None | None here; the session folder is imported on a stronger machine | None |

RTF (real-time factor) is processing time divided by audio time: 0.5 means an hour of audio takes 30 minutes.

**Published data points** (other machines; the system check measures the real ones):

| Model and settings | Hardware | Result | Source |
| --- | --- | --- | --- |
| small, beam 5, 13 minutes of audio | Core i7-12700K, 8 threads | whisper.cpp 2 min 05 s (RTF about 0.16); whisper.cpp with OpenVINO 1 min 45 s; faster-whisper int8 1 min 42 s, batched 51 s | faster-whisper README |
| small and medium | Apple M1, CPU only | RTF 0.17 and 0.40 | snailtext, June 2026 |
| large-v3 | Ryzen 7 5700G, CPU only | RTF about 3.0 | snailtext, June 2026 |
| Integrated GPU through Vulkan (whisper.cpp 1.8.3) | Ryzen 7 6800H (Radeon 680M), Core Ultra 7 155H (Arc) | 3 to 4 times better RTF than CPU-only on the same machine | Phoronix, January 2026 |

By parameter count, medium (769M) costs about three times small (244M), and large-v3 (1.55B) about twice medium. large-v3-turbo keeps large-v3's 32-layer encoder with a 4-layer decoder, so on a CPU it is expected to land between medium and large-v3. Older integrated GPUs (UHD, early Iris Xe) were not part of the published tests; Phase 2 and every system check measure them.

**Running it offline and fast on any of these laptops**
- Windows 11 with all updates installed while online, then Windows Update paused for the event; sleep off; best-performance power mode; mains power (laptops throttle CPU and GPU together on battery).
- Graphics drivers installed while online: integrated GPUs need a driver with Vulkan support for the hybrid path.
- A line-level feed from the hall mixer through a USB audio interface, never the laptop microphone; speech peaks near −12 dBFS.
- An SSD with at least 50 GB free and a second drive for the mirror.
- Close other apps: the post-processing engines use most CPU threads and the GPU at low priority.
- The Legion can go to any room; the Start Session dialog offers live text there because its profile clears it.

## 3. First-launch system check

The app runs the check on first launch, whenever the hardware, drivers or model pack change, and on demand from the System screen. The result is a capability profile (`capability.json`) that every screen reads.

1. **Probe (seconds).** Windows version; CPU model, physical and logical cores, instruction sets (AVX, AVX2, FMA, F16C, AVX-512); RAM total and free; GPUs (NVIDIA through NVML; every adapter through DXGI, including integrated GPUs and their shared memory); free space on the recording and backup drives; mains or battery; audio inputs.
2. **Engine sanity.** Each bundled engine build (`avx512`, `avx2`, `vulkan`, `cuda`, and `openvino` if shipped) runs `info` in its own process with a timeout. A build that crashes (for example on a CPU without AVX-512) or finds no device is marked unusable; the app is unaffected.
3. **Speed test.** On bundled 60-second clips per language, plus one code-switched English–Hindi clip, each candidate that fits in memory runs exactly as it would be used, on each usable device configuration: CPU only; GPU only; CPU and GPU together (two engines on separate work units); and, on Intel GPUs with the `openvino` build, GPU encoder with CPU decoder. Near-live candidates run greedy on utterance-length chunks with half the logical threads; rolling and night candidates with beam 5. The check records throughput, RTF, peak memory (including GPU shared memory) and load time, and tries two thread counts on hybrid CPUs. The quick check on first launch takes about 5 to 8 minutes on a low-baseline laptop; the full check up to 40.
4. **Recorder safety.** The chosen configuration runs for 2 minutes alongside a test recording; any recorder overrun rules it out.
5. **Decide.** Apply the rules below and estimate when a 90-minute talk's transcript would be ready.
6. **Save** `capability.json` with a fingerprint (CPU, cores, RAM, GPUs and drivers, app version, model manifest hash), every measurement with its device configuration, method and thread count, the verdict and any overrides.
7. **Explain.** The System screen shows the verdict in plain words. The Start Session dialog reads it before every recording.

| Choice | Rule |
| --- | --- |
| Post-processing configuration | The configuration with the highest measured throughput for the rolling model that passes the recorder safety run and keeps at least 3 GB RAM free (shared GPU memory included) |
| Tier G | A GPU engine works and the live model runs at RTF ≤ 0.5 in streaming settings |
| Near-live for a language | The language has a validated near-live model (Phase 5), it runs at RTF ≤ 0.35 on half the logical threads, and at least 3 GB RAM stays free with the near-live and rolling models loaded |
| Live offered for a session | Every expected language of the session qualifies (tier G for streaming, near-live rule on tier C+); otherwise the dialog shows live as unavailable, with the reason |
| Rolling model for a language | The most accurate validated model with RTF ≤ 1.0 in the chosen configuration; if none qualifies, the fastest validated model, and the screens show the expected lag |
| Night model for a language | The most accurate validated model whose total time for that day's recordings fits between the last session and 08:00; skipped if it is not more accurate than the rolling model |
| Threads | Post-processing: physical cores minus one in total across engines, halved during live sessions. Near-live and live: half the logical threads |
| Record only | No AVX2, under 8 GB RAM, or no validated model fits in memory |

"Most accurate" always means the Phase 5 ranking on the local STELMAR test sets (including code-switched speech), never a published number.

## 4. Modes, the Start Session dialog and the post-processing queue

**Before each recording** the Start Session dialog asks two things:
1. **Languages expected:** English and Hindi ticked by default, Urdu optional. Detection runs among the ticked languages only; ticking one locks it.
2. **Mode:** Record + post-process is selected by default on every machine. Live + post-process is shown but disabled, with the reason, unless the profile clears it for every ticked language. The dialog also shows where post-processing will run (for example "CPU + Iris Xe") and the estimated ready time.

| Mode | During the talk | After the talk |
| --- | --- | --- |
| Record + post-process (default everywhere) | Records; the rolling pass transcribes each closed 10-minute part in the background, about one part behind the room | The last part is transcribed; the night pass may later replace the text with a more accurate version |
| Live + post-process (only where cleared and chosen) | Records; live text on the Record screen and the caption window | Post-processing produces the transcript people review and export; the live text stays as a draft |

The Record screen's **Live captions** switch can turn live text off at any time without interrupting recording; turning it on mid-session follows the same rules as the dialog.

**Queue rules**
1. Jobs: rolling (one per session, follows the recorder), night (per day), translation (optional) and import (session folders from other machines).
2. Priority: recording, then live, then rolling, then night, then translation.
3. Work units: each job is cut into units of about 5 minutes that start and end at VAD pauses. Units can run on different engines at once (CPU and GPU in the hybrid configuration); the app writes their segments in audio order, so the transcript never jumps.
4. Engines start at below-normal priority with the profile's devices and thread counts. They pause when free RAM drops under 2 GB, on battery power, when the recorder reports any overrun, and during live sessions on CPU-only machines. They resume by itself.
5. Each job keeps a cursor (`committed_until_ms`) up to which every unit is committed. The rolling pass covers audio up to 2 seconds before the end of the last closed part; at Stop it finishes the session. The app is the only writer of transcript files and fsyncs each segment before display. After a crash, jobs resume from the cursor without losing or repeating a segment.
6. The Queue screen shows each job's progress, model, devices, threads and estimated finish, with Pause, Resume, Move Up and Change Model.

**When is the transcript ready?** With 10-minute parts, a 90-minute talk's transcript is complete at about max(10 + 90 × RTF, 90 + 10 × RTF) minutes after the talk starts, where RTF is that of the whole configuration:

| Rolling RTF | Transcript complete after the talk ends |
| --- | --- |
| 0.5 | about 5 minutes |
| 1.0 | about 10 minutes |
| 1.5 | about 55 minutes |
| 2.0 | about 100 minutes |

Each room records at most about 3 hours a day on the STELMAR programme, so even a night model at RTF 2.5 clears a day's audio well before 08:00.

## 5. Models and languages

English and Hindi are the primary languages, and STELMAR speakers will switch between them: whole passages in one language, a sentence or two in the other, or English terms inside Hindi sentences. Urdu is supported for speakers who use it.

**Code-switching**
1. **Sentence-level switches.** In sessions with more than one expected language, audio is cut at pauses into sentence-sized chunks (about 3 to 15 seconds). Whisper's language detection runs on each chunk, restricted to the session's languages, and the chunk is decoded in the detected language with that language's model. A chunk shorter than 2 seconds takes its neighbours' language unless detection is at least 0.8 sure.
2. **Uncertain chunks.** When the top two languages are within 0.2 of each other, the chunk is decoded both ways and the result with the higher average log-probability is kept. This doubles the cost only for those chunks.
3. **No silent translation.** Whisper forced into the wrong language tends to translate instead of transcribe. A chunk is never decoded as English when detection says another expected language with 0.5 or more, and an output whose script contradicts its language is re-decoded.
4. **Mixing inside a sentence (Hinglish).** Decoded with the Hindi or code-switching model. Hindi words are written in Devanagari; English words in Latin script as spoken (D14) when the chosen model does this, as the Hinglish fine-tune below does; otherwise as the model writes them.
5. **Routing or one model.** Phase 5 compares routing each chunk to a per-language model (English model for English chunks, Hindi model for Hindi chunks, both loaded) with a single multilingual or code-switching model, on the local code-switched set, and keeps the more accurate one that fits the machine.
6. **Operator control.** Alt+1/2/3 lock English, Hindi or Urdu from the next chunk on (for example during a long single-language passage); Alt+4 returns to detection. Every change is logged with its audio time, and post-processing follows the log.

Plain Whisper models below large are weak in Hindi and Urdu, so CPU machines rely on Whisper models fine-tuned on Indian speech.

| Evidence (WER %) | Small | Medium | Large |
| --- | --- | --- | --- |
| Urdu, zero-shot: read speech (ARL, CSaLT); conversational | 48.7, 41.1; 55.7 | 37.0, 33.4; 40.2 | 26.3, 24.4; 18.3 |
| Urdu, after fine-tuning (same sets) | 28.6, 27.4; 32.9 | 25.4, 24.2; 28.9 | 23.8, 22.4; 17.9 |
| Hindi, Common Voice 11 | 63.5 zero-shot; 14.1 fine-tuned (whisper-hindi-small) | 11.4 fine-tuned (whisper-hindi-medium) | 30.8 zero-shot (large-v3); 12.9 Hinglish fine-tune |
| Code-switched Hindi–English (CoSHE-500, conversational) | Not measured | Not measured | 29.7 zero-shot (large-v3); 13.7 Hinglish fine-tune |

Sources: WER We Stand (Urdu, Table 3); Hugging Face fine-tuning blog (Hindi zero-shot small); vasista22 model cards; Trelis whisper-hinglish-preview card (large-v3 rows, script-safe normaliser).

Candidates (Phase 5 picks per language and role; sizes are whisper.cpp ggml files):

| Model | Base | ggml size | Licence | Candidate role | Published result |
| --- | --- | --- | --- | --- | --- |
| Whisper small | small (244M) | 466 MiB | MIT | English near-live on CPU | Not used for Hindi or Urdu |
| whisper-hindi-small (vasista22) | small | about 466 MiB after conversion | Apache 2.0 | Hindi near-live on CPU | Common Voice 11 14.1%; FLEURS 9.0% (trained on FLEURS train and dev, so not independent) |
| whisper-hindi-medium (vasista22) | medium (769M) | about 1.5 GiB | Apache 2.0 | Hindi rolling | Common Voice 11 11.4%; FLEURS 6.8% (same caveat) |
| IndicWhisper Hindi (AI4Bharat Vistaar) | medium | about 1.5 GiB | MIT (confirm in Phase 5) | Hindi rolling | Kathbath 10.3%; FLEURS 11.4% |
| whisper-medium-vaani-hindi (ARTPARK-IISc) | medium | about 1.5 GiB | Apache 2.0 | Hindi rolling | FLEURS 14.3%; Kathbath 14.3% |
| whisper-hinglish-preview (Trelis) | large-v3 (from the Vaani large-v3 checkpoint) | 2.9 GiB (q5_0 about 1.1 GiB) | Apache 2.0 | Hindi and code-switched night pass; rolling where a GPU makes it fast enough; GPU live | CoSHE-500 13.7%, FLEURS-hi 12.6%, FLEURS-en 6.9% (large-v3: 29.7, 27.5, 4.8) |
| IndicWhisper Urdu (AI4Bharat Vistaar) | medium | about 1.5 GiB | MIT (confirm in Phase 5) | Urdu rolling | Kathbath 11.9%; FLEURS 19.2% |
| Whisper large-v3-turbo | 809M | 1.5 GiB (q5_0 547 MiB) | MIT | English rolling; GPU live | Hindi FLEURS 29.6% in a vendor test, and it cannot translate, so not for Hindi or Urdu |
| Whisper large-v3 | 1.55B | 2.9 GiB (q5_0 1.1 GiB) | MIT | GPU live; night pass; translation | Urdu 18.3% conversational, 24.4 to 26.3% read |
| Silero VAD v6.2.0 | | Small | MIT | Voice activity detection | Built into whisper.cpp |

- **Quantisation.** q5_0 cuts a ggml file to about 37% of its size (large-v3: 2.9 GiB to 1.1 GiB) and speeds up CPU decoding, but it can cost more accuracy in low-resource languages. Phase 5 tests q8_0 and q5_0 for every candidate and ships q5_0 only when its WER stays within 1 point of q8_0.
- **Fine-tunes and English.** Hindi fine-tunes can lose English accuracy (the Hinglish model is 6.9% on FLEURS-en against 4.8% for large-v3), which is why per-chunk routing is evaluated.
- **Urdu.** No small Urdu model is good enough for near-live text yet, so Urdu is never offered live on CPU machines. A licensed small Urdu fine-tune can join if it passes Phase 5.
- **Scripts.** Hindi in Devanagari; Urdu in Perso-Arabic script, right to left, in Nastaliq; English in Latin. Every segment stores `lang`, `lang_prob`, `mixed` and `dir`; mixed segments render with `dir="auto"`.
- **Timestamps** come from the audio clock and VAD, never from model timestamps.
- **The audio clock** (decided 9 Nov 2026, Phase 1): segment times are milliseconds of recorded audio from session start, continuous across parts, with paused time and input-loss gaps excluded. `session.json` logs each pause and resume with its wall-clock time (IST) and its audio time, so the UI can show both.
- **Not in version 1:** distil-whisper (English only); faster-whisper as a second engine (about twice as fast on CPU with batching in its README, but it needs a bundled Python runtime; the engine protocol allows adding it later); Qwen3-ASR (no Urdu); SeamlessM4T (non-commercial licence).

## 6. Accuracy plan

Accuracy comes from seven things, in this order: a clean line feed, the right language for every chunk, the right fine-tuned model per language, voice detection before Whisper, careful decoding in post-processing, a night pass where it helps, and human review.

| Setting | Near-live (CPU) | Live (GPU) | Rolling and night passes |
| --- | --- | --- | --- |
| Language | Detected per utterance among the session's languages | Detected per utterance among the session's languages | Detected per sentence-sized chunk; uncertain chunks decoded both ways |
| Decoding | Greedy | Greedy partials, beam 5 finals | Beam 5 with temperature fallback |
| Audio window | One VAD utterance (up to 15 s), `audio_ctx` sized to it | 1-second steps over the current utterance (up to 25 s) | Sentence-sized chunks (3 to 15 s) in multi-language sessions, up to 30 s otherwise |
| Context | None | Last 200 characters of committed text in the same language | Previous segment text in the same language, reset after a flagged segment |
| Prompt | Glossary | Glossary | Glossary (STELMAR, BML Munjal University, speaker names, track keywords, in Latin script) |
| Result | Draft revision `live-1` | Draft revision `live-1` | Transcript revisions `post-1` (rolling) and `post-2` (night) |

1. **Guards.** (Changed 10 Nov 2026; the rule before was "drop a segment when the compression ratio is over 2.4", measured on UTF-8 bytes as whisper does.)
   - Drop a chunk, with a `segment_dropped` warning event, when the VAD speech ratio is under 0.3, or when `no_speech_prob` is over 0.6 with average log-probability under −1.0.
   - A chunk that looks like a loop is never dropped at once. A loop is a chunk whose script-independent compression ratio is over 2.64, or that reached the token limit. The ratio is the number of characters divided by the zlib-compressed length after mapping each distinct character to one byte, so Devanagari, Urdu and Latin text are measured alike. The engine then climbs a ladder: (a) decode the chunk again without previous-text context, starting at temperature 0.4, then 0.6, then 0.8 with temperature fallback on (whisper.cpp's own fallback never fires when our loop check is what failed, and a decode at temperature 0 repeats the first one exactly; changed 11 Nov 2026 after run 2 showed step a rescuing nothing); (b) split it at its deepest VAD pause (at least 0.5 s or one fifth of the chunk from each end) and decode each half without context; (c) a part that still fails becomes a segment with flag `unclear`, its start and end times and no transcript text, so review shows the gap; the original chunk text travels beside it as `hint_text` so a reviewer can see what the model heard (changed 11 Nov 2026 by the owner, because a bare gap gave the reviewer nothing to correct from; `hint_text` is for review only and never enters the transcript, exports, captions or WER). A chunk is never lost without an event: `chunk_retry`, `chunk_rescued` (naming step a or b) or `chunk_unclear`. Rescued segments carry the flag `retried`. Unclear chunks count as truncated for gates.
   - Why: UTF-8 spends three bytes on each Devanagari or Arabic-script letter, so ordinary Hindi looks repetitive to a byte measure. The reference transcript of CoSHE-Eval clip audio_1444 scores 2.43 on whisper's measure, and the engine dropped real Hindi speech from it while real loops (audio_908, audio_37) were dropped instead of retried.
   - Calibration: the threshold 2.64 is the geometric middle between the highest ratio of any reference transcript (2.55: CoSHE-Eval, FLEURS en, hi and ur; windows of 25 words reach 2.0 and of 50 words 2.2) and the lowest known loop (2.74); the numbers are in `docs/evidence/phase-2/guard-calibration.txt`. Re-run `scripts/gates/guard_calibration.py` before changing it.
   - Coverage check (added 11 Nov 2026, approved by the owner). A decode can be confident and not a loop and still miss a stretch of speech (one 14 s Hinglish chunk came out as 43 characters and no guard fired). A chunk with 3 s of speech or more whose letters and digits per second of VAD speech are under the floor of its language is treated as a failure and goes through the same ladder. If it is still under the floor after the ladder, its text is kept and flagged `low_coverage` (counted in `low_coverage_chunks`; never dropped, never shown as a caption without the flag), so review shows it; if no attempt produced any text it becomes `unclear`. A chunk that decodes to nothing and is under 3 s of speech gets a `chunk_empty` warning event. The floor of each language is 0.9 times the lowest rate in the reference transcripts (CoSHE-Eval, FLEURS en, hi, ur), in `guards::COVERAGE_FLOOR`; the calibration is `docs/evidence/phase-2/coverage-calibration.txt` (11 Nov 2026: floors en 5.79, hi 5.03, ur 7.70 letters and digits per second of speech; the lowest reference rates are 6.43, 5.59 and 8.56; the known short chunk is at 3.34 and is caught; no other accepted chunk of the Hinglish run is under the floor).
   - Repeated n-grams, known silence phrases ("Thanks for watching" and similar) and output in the wrong script are flagged for review and never reach captions.
2. **Evaluation (Phase 5).**
   - Data: FLEURS test (en_us, hi_in, ur_pk; CC BY 4.0), Kathbath test (hi, ur), CoSHE-Eval (code-switched Hindi–English; CC BY-NC 4.0, evaluation only), and local STELMAR sets recorded through the venue chain and transcribed by hand: 15 minutes each of English, Hindi and Urdu, plus 30 minutes of natural code-switched English–Hindi talk.
   - Metrics: WER for all; CER as well for Hindi and Urdu; language-detection accuracy per chunk on the code-switched set. Script-aware normalisers (Hindi NFC with vowel signs kept and nukta forms unified; Urdu NFC with ي/ی, ك/ک, ه/ہ unified and optional diacritics removed; for mixed segments each word normalised by its script). Whisper's default normaliser strips Indic vowel signs and flatters Hindi WER by 10.7 to 34.1 points, so it is never used.
   - Speed: throughput, RTF and peak memory on the low-baseline and mid laptops (each device configuration) and the Legion, measured by `wb-engine bench`.
   - Every candidate runs through `wb-engine batch` with its role's exact settings, so the numbers match the app.
3. **Choice rules.** Rolling: lowest local WER (code-switched set weighted equally with the single-language sets) with RTF ≤ 1.0 on the low-baseline laptop's best configuration. Night: lowest local WER overall. Near-live: local WER no more than 1.5 times the language's rolling WER, and RTF ≤ 0.35. Ties go to the smaller model.

## 7. Architecture

```text
Venue mixer ──► USB audio interface ──► capture (cpal, 48 kHz) ──► ring buffer
                                                     ├──► Recorder: WAV parts + parts.json ──► live mirror to backup drive
                                                     └──► live mode only: 16 kHz mono ──► live engine process (stdin)
Recorder "part closed" ──► Scheduler ──► work units ──► batch engine on CPU ─┐
                                                   └──► batch engine on GPU ─┴──► segments (in audio order)
Engines ──► JSON lines ──► App, the single writer: SQLite (WAL) + JSONL revisions
                             ├──► Record screen and caption window
                             ├──► Queue screen
                             └──► Review, export, archive
Everything runs on one machine. Nothing touches the network.
```

- **App process (Tauri, Rust):** capture, recorder, mirror, store, scheduler, system check, engine supervisor and UI. It never links whisper.cpp, so a model crash, an unsupported CPU instruction, a GPU driver fault or running out of memory cannot stop the recording.
- **Engine process (`wb-engine`):** whisper.cpp through whisper-rs 0.16 with Silero VAD. Subcommands: `info` (CPU features, devices), `bench` (speed and memory per device), `batch` (transcribe work units, or a single file for evaluation) and `live` (16 kHz PCM on stdin). It speaks versioned JSON lines on stdout and logs on stderr, and can hold two models (one per language) for per-chunk routing.
- **Builds:** `avx2` (portable baseline), `avx512`, `vulkan` (any GPU with a Vulkan driver, including integrated ones), `cuda` (NVIDIA) and, experimentally, `openvino` (encoder on Intel GPUs, decoder on the CPU).
- **Hybrid post-processing:** in the CPU + GPU configuration the scheduler runs a `vulkan` (or `cuda`) engine and a CPU engine at the same time on different work units, splitting threads so the GPU engine keeps one or two for its decoder. Integrated GPUs share RAM and the laptop's power budget with the CPU, so the combination is used only where the system check measured it faster.
- **Priorities:** recorder thread raised (MMCSS "Pro Audio"); live engine normal; batch engines below normal. All engines sit in a Windows Job Object, so they close with the app.

## 8. Recording and backup

Audio is written to disk before anything else touches it.

1. **Capture:** cpal at the device rate (48 kHz) copies samples into a lock-free ring buffer holding 10 seconds or more. The audio callback does nothing else.
2. **Recorder thread:** WAV parts of 10 minutes; `hound::WavWriter::flush()` every second keeps each file playable; fsync every 5 seconds. When a part closes, its start time, duration and SHA-256 are appended to `parts.json`, which the post-processing cursor and the Review player follow.
3. **Mirror thread:** keeps the same files on the backup drive.
4. **Recovery:** at start-up, sessions without `closed_at` get WAV headers repaired from file length, `parts.json` rebuilt and transcripts replayed from JSONL; recording can resume.

```text
D:\STELMAR-Transcripts\
└─ 2027-02-08\
   └─ hall-a__inaugural-session-inaugural-keynote\
      ├─ session.json        title, hall, times, expected languages, language log, mode, models, devices, app version, opened_at, closed_at
      ├─ parts.json          closed parts: start_ms, duration_ms, sha256
      ├─ audio\part-0001.wav 48 kHz 16-bit, 10 minutes each
      ├─ live-1.jsonl        live or near-live draft (live mode only)
      ├─ post-1.jsonl        rolling pass, append-only
      ├─ post-2.jsonl        night pass, if it ran
      ├─ transcript.db       SQLite: segments, revisions, edits, bookmarks, speakers, jobs, work units
      ├─ exports\            .txt .srt .vtt .json .docx
      └─ manifest.sha256     hashes of every file at session close
```

- **Segment fields:** id, start_ms, end_ms, lang, lang_prob, mixed, dir, text, speaker, model, device, revision, avg_logprob, no_speech_prob, flag, created_at.
- **Storage:** 48 kHz 16-bit mono is about 346 MB per hour, so about 2 GB per room for the two days, plus about 6 GB of models.
- **Bookmarks and speaker tags** are logged with audio-clock times during recording and attach to the transcript when it arrives.

## 9. Screens and design consistency

The app looks like the STELMAR website because it uses the same files: the five token CSS files copied unchanged, components ported from the design-system bundle, the charcoal header with the orange underline, Inter, and IBM Plex Mono for times, file and model names. Reference mockups: `design/mockups/start-session.html`, `record-only.html`, `record-live.html`, `system-check.html` and `captions.html` (serve the kit folder with any static server; browsers block fonts over `file://`). Numbers in the mockups are placeholders.

| Screen | Purpose | Built from |
| --- | --- | --- |
| Sessions | Programme by day (seeded from `docs/programme.json`), status and mode per session; New Session | SessionItem, Badge, Button |
| Start Session dialog | Before every recording: expected languages, mode (record + post-process by default; live only if cleared), where post-processing runs, estimated ready time | `start-session.html`; Dialog, Checkbox, radio-style Tags, Button |
| Record | Clock, recording state, input meter, languages, backup; rolling transcript (record mode) or live text (live mode); the Live captions switch | `record-only.html`, `record-live.html` |
| Queue | Post-processing jobs: progress, model, devices, threads, estimated finish; Pause, Resume, Move Up, Change Model | SessionItem-style rows, Badge, Dialog |
| Review | Edit the transcript against the audio; revisions, speakers, bookmarks, language per segment, search | Tabs, Input, Tag, Toast |
| System | System check verdict, hardware, speed test per device configuration, models and checksums, overrides; Run Full Check | `system-check.html`, `.tbl` |
| Settings | Folders, backup drive, glossary, caption style, hotkeys, scheduler limits | Input, Select, Switch |
| Captions | Second window on the projector (live mode only) | `captions.html` |

- **Tokens:** `design/tokens/{colors,typography,spacing,effects,base}.css` are frozen copies. Additions go only in `app-additions.css`.
- **Components:** port Button, IconButton, Icon, Badge, Tag, Input, Select, Checkbox, Switch, Card, Tabs, Dialog, Toast, Tooltip and SessionItem from `design/reference/ds-bundle.js` with identical props and styles. The progress bar is the only new component, built from tokens. No other UI kit.
- **Header:** charcoal bar (`--surface-header`) with the logo lockup; nav at 15px weight 500; active item white with the orange underline.
- **Type:** Inter; IBM Plex Mono only for times, durations, file names and model names; Hindi in Noto Sans Devanagari; Urdu in Noto Nastaliq Urdu with Noto Naskh Arabic as fallback, right to left; mixed Hindi–English segments keep each word in its own script. All fonts are local files (SIL OFL).
- **Colour, motion and voice:** charcoal and white, with the logo spectrum only as small accents; 120, 200 and 320 ms fades; Title Case buttons, no emoji, Indian and British spelling, times in IST.
- **Adherence lint:** the design system's own `design/reference/adherence.oxlintrc.json` runs in CI, extended only to allow the three Noto families.

## 10. Offline guarantee

Nothing in the app or its engine processes may reach the network. Phase 8 proves it twice: once watched on a connected network, where any connection attempt would show, and once with the network off, as on event day.

- **No runtime downloads:** models, fonts, icons, test clips and the WebView2 runtime (offline installer mode) are installed beforehand.
- **Tauri:** no HTTP, updater, shell or telemetry plugins; engines are started from Rust with `std::process`; capabilities allow only the app's own commands; a strict Content Security Policy.
- **Engines:** no networking crates in `wb-engine`, checked with `cargo tree`; GPU and OpenVINO runtimes run locally and are covered by the proof test.
- **Integrity:** model files are checked against SHA-256 values in `models/manifest.json`; a mismatch disables that model.
- **CI checks:** fail when the built UI contains http or https URLs other than local IPC, or when `Cargo.lock` gains HTTP-client crates.
- **Proof test:** a scripted 30-minute session on a connected network while `Get-NetTCPConnection`, `Get-NetUDPEndpoint` and Windows Filtering Platform auditing watch the app's process tree (the app, its engines and the WebView2 processes it starts): no connection beyond loopback. Then the same session with an outbound firewall block and the network adapters off, which must run normally.

## 11. Build and packaging

1. **Portable CPU builds.** ggml optimises for the build machine unless told otherwise, and CI runners have AVX-512 while venue laptops may not. Release engines are built with `GGML_NATIVE=OFF` plus explicit flags (`avx2`: `GGML_AVX`, `GGML_AVX2`, `GGML_FMA`, `GGML_F16C`; `avx512`: those plus `GGML_AVX512`). whisper-rs-sys forwards any `GGML_*` and `WHISPER_*` environment variable to CMake, so `scripts/build-engines.ps1` sets them per build. CI scans the `avx2` binary for AVX-512 (zmm register) instructions and fails if it finds any.
2. **whisper.cpp version.** Integrated-GPU support through Vulkan improved substantially in whisper.cpp 1.8.3. Phase 2 confirms that the whisper.cpp bundled by whisper-rs-sys is 1.8.3 or later, or vendors a newer one.
3. **Runtime checks.** Each engine verifies its instruction sets with `is_x86_feature_detected!` before touching whisper.cpp and exits with a clear code if any are missing.
4. **OpenVINO (experimental).** whisper.cpp runs only the encoder on OpenVINO (`WHISPER_OPENVINO=1`), on x86 CPUs or Intel GPUs. whisper-rs has no feature for it, so the `openvino` build sets the variable, calls `whisper_ctx_init_openvino_encoder` through `raw-api`, bundles the OpenVINO runtime (Apache 2.0) and converts each model's encoder at development time. It ships only if Phase 4 shows it beats the Vulkan path on Intel laptops.
5. **Sidecars.** Engines ship as Tauri `externalBin` files (`wb-engine-avx2-x86_64-pc-windows-msvc.exe` and so on) beside the app.
6. **Installer.** NSIS and MSI from `pnpm tauri build`, with `bundle.windows.webviewInstallMode` set to `offlineInstaller` (about 127 MB more). The CUDA engine's runtime DLLs ship only where NVIDIA's licence allows redistribution; otherwise the CUDA build is an optional install step, and the Vulkan build covers GPUs.
7. **Model packs.** A CPU pack (the validated small and medium-class models and turbo) and a GPU pack (adds large-v3 and the code-switching model), each with `manifest.json` and an Install Model Pack action that verifies hashes.

**Minimum to run:** Windows 11, x86-64 CPU with AVX2, 8 GB RAM (16 GB recommended), 20 GB free disk plus about 350 MB per hour of recording, and an audio input. A Vulkan-capable graphics driver lets the integrated GPU help.

## 12. Phases and gates

Each phase has its own branch and prompt. The next phase starts only after the gate passes and the reviewer subagents report PASS. "Low-baseline laptop" means the weakest 16 GB laptop you can borrow (4 cores, basic integrated graphics); "mid laptop" a newer one with a stronger integrated GPU; "Legion" is the build and GPU test machine.

| Phase | Delivers | Gate before the next phase | Needs a person or machine |
| --- | --- | --- | --- |
| P0 | Tauri 2 and React shell in the STELMAR design, six screens, first-launch route, Cargo workspace, CI | Shell matches the mockups; adherence lint clean; opens with no network | Windows machine |
| P1 | Audio capture, meter, crash-safe WAV recorder, parts index, mirror | 2-hour soak and a 30-minute full-CPU-and-GPU-load test with zero overruns; force-kill leaves playable files; mirror hashes match | Low-baseline laptop with a virtual audio cable |
| P2 | Engine processes per backend, protocol, model registry, supervisor | Portable builds run on both laptops; Vulkan works on their integrated GPUs; en, hi, ur and code-switched clips transcribe; speed table per device; engine kills lose nothing | Both laptops and the Legion |
| P3 | System check with device configurations, capability profile, System screen, Start Session dialog | Verdicts follow the rules on all three machines; quick check under 8 minutes on the low-baseline laptop | Both laptops and the Legion |
| P4 | Post-processing queue: work units, hybrid CPU + GPU, rolling and night passes, scheduler, Queue screen, import | Day simulation with code-switched sessions: zero recorder overruns; transcripts ready as estimated; hybrid used only where faster; resume after 10 engine kills | Both laptops |
| P5 | Language detection and code-switching, Hindi and Urdu rendering, conversion and quantisation, evaluation, model choice per role | `eval/REPORT.md`; chosen models beat zero-shot medium on the local sets, including code-switched speech; manifest defaults set | Local test sets transcribed by hand |
| P6 | Sessions, Review editor, revisions, exports, archives | Edits survive a restart; SRT and VTT parse; mixed-script text and right-to-left Urdu survive every export; archive verifies and re-imports | A person checks exported files |
| P7 | Live (GPU streaming) and near-live (CPU) text behind the Start Session dialog, caption window, translation on GPU | Off by default; GPU p95 partial ≤ 1.5 s; CPU near-live p95 ≤ 6 s; post-processing stays on schedule | Legion, the mid laptop if it qualifies; a person reads captions in a hall |
| P8 | Hardening, offline proof, day simulation, installers, model packs, operator documents | Watched run clean; day simulation targets met on both laptops; offline install on a clean laptop | Clean laptop; hall dry run with sign-off |

**Phase 1 recording-gate rule (9 Nov 2026).** The Phase 1 recorder gates (soak, load, panic, WebView2 kill and the force-kill test) judge the app against what Windows delivered, not against the source signal. A virtual audio cable and the Windows audio engine sit between the source and every capture client, and on the Legion they sometimes repeat one block (about 500 to 580 frames, one WASAPI period) that the player, the app and a second capture client then all record identically; no recorder can prevent or see that. So each run has an independent reference capture of the same cable (a separate cpal + hound recorder), and:

- **Fidelity (decides PASS or FAIL).** The app's recording equals the reference capture sample for sample over the whole run, after one fixed alignment offset found at the start of each run of the app and never adjusted later (a force-killed and restarted session has one run per start, each with its own offset), and `dropped_samples` is 0. Any difference is a FAIL. Reference audio after the app's last frame is a difference; audio before the app's first frame, or between the runs of a force-killed session, is not the app's to record. The alignment snippet is taken from the signal itself (its start comes from the source analysis), because the cable can hold stale audio before it that matches itself at many offsets.
- **Upstream breaks (reported, not gated).** Breaks between the source and the reference capture. Each is listed with its source position, wall-clock time, repeat and unmatched lengths, the nearest reference xrun and the player's late callbacks within 500 ms. A late callback is a gap between the player's render callbacks longer than its 100 ms output buffer.
- **Validity (INVALID means no verdict and the run is repeated).** A run is INVALID if any upstream break has a late player callback within 500 ms, or has more than 250 ms unmatched, or if there is more than 1 upstream break per 10 minutes of audio, or if the reference capture is missing, misaligned, incomplete or lost samples itself. (As amended by Amendment 1 below: late-callback data is required only for a run that has an upstream break, and the break limit is a count per run.)
- **App xruns.** The start-up xrun rule for the app's own xruns is unchanged: one driver xrun within 2 s of a stream start is reported, not gated, only with zero loss or duplication around it.

**Amendment 1 to the Phase 1 recording-gate rule (9 Nov 2026).** The first re-scoring showed two specification errors and one inconsistency; the fidelity check (the app equals the reference capture, one alignment per run of the app, 0 dropped) does not change. The final Phase 1 verdicts come only from a fresh run of every gate under this amended rule; earlier re-scored runs are information only.

- **(a) Late-callback data is required only for a run that has an upstream break.** A run with 0 upstream breaks needs none. *Reason:* the data exists to attribute a break; with no break there is nothing to attribute.
- **(b) The break limit is a count per run, not a rate:** at most max(1, floor(minutes of audio / 10)) counted upstream breaks (1 for a run under 20 minutes, 3 for 30 minutes, 12 for 2 hours). *Reason:* a rate made a single break in a 3-minute run INVALID (3.3 per 10 minutes).
- **(c) Upstream breaks within the first 3 s of playback are reported but not counted toward (b).** The 250 ms unmatched limit and the late-callback condition still apply to them. *Reason:* consistent with the start-up allowance for the app's own xruns; the cable replays stale buffer audio at start-up, which is also why the alignment snippet is taken from the signal.

Where the repeat comes from (the cable driver or the audio engine) is not established; repeated-block detection in the app is the only defence against it on real hardware and is a Phase 2 item (`docs/GATES.md`). Implemented in `crates/wb-testbench/src/compare.rs` and `scripts/lib/Fidelity.psm1`, with tests for each outcome.

**Before the laptops arrive.** The test laptops are not available before 17 November 2026. Until then the Legion stands in for them: `crates/wb-emulate` runs the app and engines in a Job Object limited to 4 efficiency cores (`emu-low`) or 2 performance and 8 efficiency cores (`emu-mid`), 11 GB of memory and the Intel integrated GPU, with the NVIDIA GPU hidden. Laptop gates passed this way are PROVISIONAL, need 25% speed headroom, and are re-run on the laptops with the gate pack. Phase 5a (evaluation groundwork) runs in parallel. Details, a day-by-day plan and the gate ledger are in `docs/LEGION-FIRST.md` and `docs/GATES.md`.

## 13. Using this kit with Claude Code

1. Create an empty Git repository, unzip the kit into it and commit.
2. Open Claude Code in the repository on the Legion. It loads `CLAUDE.md`, which imports `AGENTS.md`.
3. Type `/run-phase 0`. Claude reads the phase prompt, proposes a plan and waits; approve it, let it work, then check the Verification list and the screenshots before merging.
4. Repeat for phases 1 to 8 in order. Gates that need the laptops come with a PowerShell script to run there. Reviewer subagents: `design-consistency-reviewer` after UI changes, `asr-quality-auditor` after engine, model, scheduler, system-check, language or evaluation changes, `offline-auditor` before every merge.

## 14. Human-only setup

Claude Code can't do these. Each item names the first phase that needs it.

- [ ] Borrow two 16 GB test laptops (available from 17 November 2026 at the earliest; until then see `docs/LEGION-FIRST.md`) that span the likely range: a low-baseline one (4 cores, basic integrated graphics) and a mid one (newer integrated GPU such as Iris Xe, Arc or Radeon 680M/780M). Install graphics drivers with Vulkan support while online (Phase 1).
- [ ] When the rooms' machines are known, run the full system check on each during setup week; no code changes are needed for new machines (Phase 8).
- [ ] Build machine (the Legion): Git, Rust (rustup, stable), Node.js LTS and pnpm, Visual Studio Build Tools (C++ workload), CMake, LLVM (for libclang), the Vulkan SDK, and the CUDA Toolkit at a version the bundled ggml supports; the OpenVINO toolkit only if the experimental build is tried (Phase 0; Vulkan and CUDA by Phase 2).
- [ ] Install a virtual audio cable (for example VB-CABLE) on both laptops and the Legion for replay and soak tests (Phase 1).
- [ ] Buy or borrow a USB audio interface and cables for each room's mixer, plus an external SSD per machine (Phase 1 for testing).
- [ ] Download the models once and keep them on a USB drive with the manifest; Phase 5 converts and quantises the fine-tunes (Phase 2).
- [ ] Later, once most of the development work is done (after Phase 4): record the local test sets through the venue chain and have them transcribed by hand: 15 minutes each of English, Hindi and Urdu, and 30 minutes of natural code-switched English–Hindi talk, transcribed with Hindi in Devanagari and English in Latin script (needed for the local-set parts of Phase 5; see `docs/TEST-AUDIO.md`).
- [ ] Decide where the Legion goes; any room works (Phase 7).
- [ ] Tell speakers and attendees that sessions are recorded and transcribed; agree how long raw audio is kept (before the event).

## 15. Event-day runbook

1. **Setup week:** install the app and model pack on every room's machine; run the full system check on each and keep the verdicts; do a full dry run in each room with the mixer feed and a speaker who switches between English and Hindi.
2. **Each morning:** mains power, best-performance mode, Wi-Fi off and Ethernet unplugged; open the app (Offline badge green, System verdict unchanged); start the desk recorder; pick the session.
3. **Start Session dialog:** check the expected languages (English + Hindi unless told otherwise; tick Urdu if needed), keep Record + post-process unless live text is wanted and offered, check the meter, start two minutes early.
4. **During a session:** watch the recording badge and the queue lag; lock a language (Alt+1/2/3) only for a long single-language passage and return to detection (Alt+4) after; tag speakers; bookmark moments. In live mode, also watch the caption delay.
5. **If the app crashes:** reopen it; recording resumes in a new part, the queue resumes from its cursor, and the desk recorder covers the gap.
6. **After each session:** Stop. The rolling pass finishes the last part; check the mirror badge and the estimated finish on the Queue screen.
7. **End of day:** leave the machines on and on mains so the night pass can run if the profile chose one. Next morning, check that every transcript is final, copy session folders to the archive drive and verify the manifests.

## 16. Risks

| Risk | Effect | Mitigation |
| --- | --- | --- |
| Room machines weaker than expected, or different in every room | Transcripts late | Per-machine system check; models and devices chosen to keep up; night pass; import on a stronger machine |
| Unknown or older CPUs (no AVX2, or a build that uses AVX-512) | Engine crash | Separate engine processes; runtime feature checks; portable builds; record-only tier |
| Old or missing integrated-GPU drivers | No GPU help, or a GPU engine crash | Drivers installed in setup week; GPU engine isolated; CPU-only configuration as fallback |
| CPU and integrated GPU share power and memory | Hybrid slower than expected, laptop throttles | Hybrid used only where measured faster; RAM rule counts shared GPU memory; mains power and performance mode |
| Post-processing starves the recorder | Gaps in the audio | Below-normal priority; thread cap; recorder safety run in the check; pause on any overrun |
| Out of memory on 16 GB | Engine killed | Measured peak memory; 3 GB free rule; pause under 2 GB |
| Speakers switch between English and Hindi | Wrong language, translated or garbled text | Detection per sentence among the session's languages; both-ways decoding of uncertain chunks; no forced English; code-switched test set and model choice |
| English words inside Hindi sentences written in Devanagari | Harder to read and search | Code-switching model when it passes Phase 5 (D14); glossary terms in Latin script; review step |
| Small models misread Hindi or Urdu | Poor near-live text | Live off by default; fine-tuned small models only; near-live offered per language; post-processing is the record |
| Poor audio (room microphone, feedback, music) | Wrong or invented text | Line feed; VAD; guards; level check in the runbook |
| Crash or power cut | Gaps | WAV flushed every second; resumable jobs; mains power; desk recorder |
| Disk full | Recording stops | Warning at 20 GB free; refusal under 5 GB; mirror drive |
| WebView2 or Windows reaches the network | The offline promise breaks | Watched-run proof; firewall rule; network off on event day |
| Windows restarts for updates mid-session | Session interrupted | Updates installed beforehand, then paused; network off |
| A fine-tune disappoints or its licence is unclear | Model can't ship | Evaluation gate; MIT or Apache only; plain Whisper fallback |
| Recording people without notice | Complaints | Announce recording and transcription; agreed retention |
| The agent drifts from the plan | Rework | Phase gates; reviewer subagents; AGENTS.md |

## 17. Sources

- whisper.cpp README and releases (v1.9.5, 6 Oct 2026; Vulkan and OpenVINO sections): https://github.com/ggml-org/whisper.cpp
- whisper.cpp 1.8.3 integrated-GPU speed-up (Phoronix, January 2026): https://www.phoronix.com/news/Whisper-cpp-1.8.3-12x-Perf
- whisper.cpp models README (sizes, quantised files, convert-h5-to-ggml.py): https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/models/README.md
- whisper-rs 0.16 features: https://docs.rs/crate/whisper-rs/latest/features and repository: https://codeberg.org/tazz4843/whisper-rs
- whisper-rs-sys build script (forwards `GGML_*` and `WHISPER_*` variables to CMake): https://docs.rs/crate/whisper-rs-sys/latest/source/build.rs
- faster-whisper README (CPU benchmark, small model): https://github.com/SYSTRAN/faster-whisper
- Do you need a GPU for voice-to-text? (CPU data points, June 2026): https://snailtext.app/blog/do-you-need-a-gpu-for-voice-to-text/
- WER We Stand: Benchmarking Urdu ASR Models: https://arxiv.org/html/2409.11252
- Fine-Tune Whisper For Multilingual ASR (Hindi zero-shot WER): https://huggingface.co/blog/fine-tune-whisper
- vasista22/whisper-hindi-small: https://huggingface.co/vasista22/whisper-hindi-small
- vasista22/whisper-hindi-medium: https://huggingface.co/vasista22/whisper-hindi-medium
- Trelis whisper-hinglish-preview (code-switching, script per word): https://huggingface.co/Trelis/whisper-hinglish-preview
- Vistaar and IndicWhisper (fine-tuned from Whisper medium): https://arxiv.org/pdf/2305.15386 and https://github.com/AI4Bharat/vistaar
- ARTPARK-IISc whisper-medium-vaani-hindi: https://huggingface.co/ARTPARK-IISc/whisper-medium-vaani-hindi
- OpenAI Whisper README (model table, turbo translation note, MIT): https://github.com/openai/whisper
- Qwen3-ASR vs Whisper, FLEURS run (vendor test): https://whispernotes.app/blog/qwen3-asr-vs-whisper
- Pashto benchmark citing Indic normalisation effects: https://arxiv.org/abs/2604.04598
- Turning Whisper into Real-Time Transcription System: https://arxiv.org/abs/2307.14743
- Tauri releases: https://github.com/tauri-apps/tauri/releases
- Tauri Windows installer (WebView2 modes): https://v2.tauri.app/distribute/windows-installer/
- wry `with_additional_browser_args` (default WebView2 arguments): https://docs.rs/wry/latest/src/wry/lib.rs.html
- Claude Code memory, subagents and skills: https://code.claude.com/docs/en/memory.md, https://code.claude.com/docs/en/sub-agents.md, https://code.claude.com/docs/en/skills.md
