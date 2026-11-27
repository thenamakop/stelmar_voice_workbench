# Evaluation report (Phase 5, state of 27 Nov 2026)

**Status: PROVISIONAL.** Every number below was measured on the Legion (i9-14900HX, RTX 4070 Laptop 8 GB) with `wb-engine`
(whisper.cpp v1.9.5 commit d1be6fde11ac, portable builds), the language given unless a table says "detect". Speed on the room
laptops is not measured (the test laptops arrive from 17 November 2026 on; until then `emu-low` and `emu-mid` stand in, see
`docs/LEGION-FIRST.md`). `validated_roles` and `defaults` in `models/manifest.json` are not set. The owner's decisions of 27 Nov 2026 are in section 6
(night model without a discrete card, night ranks, Phase 7 live candidates). All WER and CER are in percent with the script-aware normaliser (`eval/normalize.py`; the
nukta is kept for Hindi). Every table is written from `eval/results/*/summary.json` by a script; the evidence files named in each
section are the scripts' own output (`docs/evidence/phase-5/`).

## 1. Decision rules (docs/PLAN.md section 6)

Rolling = lowest local WER with RTF at or under 1.0 on the low-baseline laptop's best configuration (the detection allowance
applies). Night = lowest local WER overall. Near-live = WER at most 1.5 times the rolling pick's WER and RTF at or under 0.35.
q5_0 only if within 1 WER point of q8_0. Rolling and night have identical decoding settings in `eval/roles.toml`, so one WER
serves both roles; they differ in which speed bar the model has to meet.

## 2. English and Hindi mixed sessions (the main case)

Local code-switched set: CoSHE-Eval (CC BY-NC 4.0, evaluation only, never committed or bundled), a tuning list of 70 clips (339 chunks) (`eval/clips/phase5-tune-coshe.txt`) and a separate held-out list
(`eval/clips/phase5-coshe-heldout.txt`). Rule d2 = the Hinglish model for every chunk; the "pair" rows use a smaller model as the language detector in front of the Hinglish model. Sources: `docs/evidence/phase-5/tune-comparison.txt`,
`tune-*.txt`, `heldout-error-kinds.txt`, `eval/results/TABLES.md`.

| Setup (rolling settings, language detected) | Tuning WER | CER | English words written in Devanagari | Chunk language correct (of 339) | Translated chunks |
| --- | ---: | ---: | ---: | ---: | ---: |
| turbo q5_0 alone (rule a) | 33.0 | 23.6 | 2.9 | 284 | 48 |
| turbo q5_0 alone (rule c, decode both ways) | 31.0 | 22.8 | 11.0 | 298 | 8 |
| Hinglish f16 (rule d2: the model for every chunk) | 17.1 | 12.6 | 11.8 | 263 | 0 |
| Hinglish q8_0 (d2) | 17.7 | 13.2 | 12.5 | 262 | 0 |
| Hinglish q5_0 (d2) | 17.0 | 12.5 | 11.8 | 260 | 0 |
| turbo q5_0 detector + Hinglish q8_0 | 28.0 | 20.7 | 1.0 | 286 | 46 |
| small q5_0 detector + Hinglish f16 (the 6 and 8 GB card pair) | 15.9 | 11.6 | 6.3 | 282 | 0 |
| small q5_0 detector + Hinglish q8_0 (the 4 GB card pair) | 16.0 | 11.6 | 6.3 | 282 | 0 |

Chunk language is information only (owner, 27 Nov 2026): what the gate counts is translated chunks and WER. Rule d1 (Hinglish f16 with the
default first-chunk rule) scored WER 38.3 and is out.

Held-out chunks: small q5_0 + Hinglish q8_0 WER 21.3, small q5_0 + Hinglish f16 21.7, Hinglish f16 alone 21.5, q8_0 alone 23.4,
q5_0 alone 36.2 (q5_0 is out: far over the 1-point rule against q8_0), turbo q5_0 alone 54.5. The owner's gate for a pair is 0
translated chunks on both sets and held-out WER at most 23.5: both pairs pass (`docs/GATES.md`, "Hinglish pair gate").
The model is the Hinglish fine-tune of large-v3 (converted by `scripts/models/`; licence and SHA-256 in
`models/manifest.local.json`, notice in `models/NOTICE-hinglish.txt`). Caveat: its training data is not stated, so it is
marked "independence unknown" in `TABLES.md`; the held-out chunks are the only evidence that is clean of tuning.

Night speed of the Hinglish files on a computer without a discrete card (`night-q8.txt`, emu-low and emu-mid): q8_0 is 1.7 to
2 times faster than f16 on the processor and 7 to 10% slower on the integrated card. For the two programme days (360 and 300 minutes of English + Hindi) it finishes wherever f16 does; in general it does not: on emu-low's integrated card f16 finishes up to 309 minutes after a 17:30 end and q8_0 up to 282, so a night of 283 to 309 minutes finishes with f16 only (Day 2, 300 minutes, is within 2 minutes of q8_0's limit of 302). Each cell is one run of one 60 s clip. **Decision (owner,
27 Nov 2026, option A): Hinglish q8_0 is the night model on every machine; f16 is not measured, chosen or shipped.**

## 3. Hindi alone and the near-live role

FLEURS hi test (418 clips) and Kathbath hi (100 clips), rolling settings, language given (`item8-live-accuracy.txt`):

| Model | FLEURS hi WER | Kathbath hi WER | near-live WER (FLEURS / Kathbath) | near-live RTF (Legion card) |
| --- | ---: | ---: | --- | ---: |
| Hinglish q8_0 | 14.0 (not independent: the starting checkpoint may have seen FLEURS) | 11.4 | 14.1 / 10.5 | 0.30 / 0.45 |
| large-v3 q5_0 (the Hindi night model, Phase 3, rolling, 20 FLEURS and 100 Kathbath clips) | 33.2 | 27.9 | not run | not run |
| turbo q5_0 | 31.0 | 31.8 | 31.4 / 32.6 | 0.11 / 0.19 |
| medium q5_0 | 39.2 | 41.7 | 42.8 / 43.6 | 0.20 / 0.26 |
| small q5_0 | 56.6 | 59.9 | 65.9 / 65.0 | 0.18 / 0.22 |

Near-live bar (at most 1.5 times the best rolling WER on the same clips, RTF at most 0.35, no truncated near-live utterance): met by the
Hinglish model on accuracy (ratio 1.00 and 0.92; only the Kathbath figures are independent) and on speed for FLEURS (0.30) but not on the
short Kathbath clips (0.45; the harness runs one engine process per clip, so engine start and model load are inside the RTF); no other
model meets it for Hindi (ratios 2.2 to 5.7). The earlier ranking that listed turbo q5_0 first for Hindi live is therefore
not supported by the local measurement. Not measured: first-word latency and the share of segments revised after commit
(`wb-engine live` reports speech boundaries only until Phase 7).

## 4. Urdu sessions

FLEURS ur (299 clips) and Kathbath ur (100 clips), rolling settings, language given (`item10-english-urdu.txt`, `item8-live-accuracy.txt`).
The output script is Urdu Nastaliq (wrong-script rate 0.0 to 0.1% for every model).

| Model | FLEURS ur WER / CER | Kathbath ur WER / CER | RTF (Legion card, FLEURS) |
| --- | --- | --- | ---: |
| large-v3 q5_0 | 20.7 / 7.3 | 19.6 / 5.6 | 0.268 |
| turbo q5_0 | 22.3 / 7.7 | 20.6 / 6.0 | 0.138 |
| medium q5_0 | 27.9 / 9.7 | 26.5 / 8.3 | 0.201 |
| small q5_0 | 38.5 / 14.5 | 34.3 / 11.2 | 0.128 |

Night: large-v3 q5_0 has the lowest WER (1.6 and 1.0 points under turbo). Rolling: turbo q5_0 is the next best and about twice as fast
on the card; whether large-v3 q5_0 keeps up on the low-baseline laptop is a speed question for the laptops. Near-live: turbo
meets the bar against the best Urdu rolling WER, which is large-v3's (ratio 1.11 and 1.10, RTF 0.12 and 0.18, no truncated chunk,
`item8-live-accuracy.txt`), with an absolute WER of about 21 to 23. Medium has one truncated near-live chunk on each set, which the
capability rules treat as a block, so turbo is the Urdu live candidate (owner's wording of 27 Nov 2026 was "turbo or medium"); small is out.
A Hindi and Urdu "one spoken language" session writes Devanagari by default (docs/GATES.md, 13 Nov 2026).

## 5. English-only sessions

FLEURS en (200-clip spread) and Svarah accented English (150-clip spread), rolling settings, language given (`item10-english-urdu.txt`):

| Model | FLEURS en WER | Svarah WER | RTF (Legion card, FLEURS) |
| --- | ---: | ---: | ---: |
| large-v3 q5_0 | 4.7 | 7.1 | 0.186 |
| medium q5_0 | 4.9 | 7.3 | 0.132 |
| turbo q5_0 | 5.2 | 7.7 | 0.114 |
| small q5_0 | 6.5 | 9.5 | 0.088 |

The first three are within 0.5 FLEURS and 0.6 Svarah WER points of each other. Small is 1.3 to 1.8 points behind turbo, which is under
the 3-point limit the owner set for choosing it on speed in a pure English session. The English near-live model is small (FLEURS
the non-quantised small model scored 5.4 on 20 FLEURS clips in Phase 3 with rolling settings; the near-live setting was not re-measured here).

## 6. Choices (owner's decisions of 27 Nov 2026 where marked; the rest provisional)

| Role | English only | English + Hindi | Urdu (with or without English) |
| --- | --- | --- | --- |
| Rolling | turbo q5_0 (rank 1; large-v3 q5_0 is 0.5 points better and 1.6 to 1.9 times as slow on the Legion card) | Hinglish pair chosen by the system check (small q5_0 detector + Hinglish q8_0, measured on the machine) | turbo q5_0 |
| Night | large-v3 q5_0, third priority (manifest night rank 1, approved) | Hinglish q8_0 redo, first priority (option A: q8_0 everywhere, f16 dropped) | large-v3 q5_0, second priority (approved) |
| Near-live (Phase 7) | small | Hinglish q8_0 only where it measures at most 0.35 RTF there, short clips included; turbo, medium, small rejected for Hindi | turbo q5_0 (medium has truncated chunks) |

Night priority order (night time is the scarcest resource; `wb_queue::night::plan_tiered`): 1 Hinglish redos of English + Hindi
and Hindi-only sessions, 2 Urdu sessions with large-v3, 3 English sessions with large-v3, each only if everything above still finishes by 08:00
IST with the 1.5 allowance; what does not fit keeps its rolling transcript and the Queue log says why. The night speed of large-v3 q5_0
on the laptops is still to be measured (the system check measures it; the gate pack records it).

## 7. Not measured, and why

* Speed on the room laptops and the final speed gates: the laptops are not here yet (docs/GATES.md, gate pack).
* The local STELMAR sets (L0 to L6): recorded by the owner; no ear-check marks or local WER yet.
* Hinglish on Urdu speech: not run (the model writes Devanagari and is not an Urdu candidate).
* First-word latency and revised-segment share for live: Phase 7.
* Medium and small at full precision (the f16 files `medium` and `small` that the manifest ranks) for English and Urdu: not run; this report measured the q5_0 files (`medium-q5_0`, `small-q5_0`), and the PLAN rule "q5_0 only if within 1 WER point of q8_0" was not checked for large-v3 or turbo (no q8_0 file exists for either).
* Hindi-only sessions (owner, 27 Nov 2026): Hinglish q8_0 is their rolling model where it meets the rolling bar on the machine and their night model (priority 1, with the English + Hindi redos) where it only fits the night, because large-v3 q5_0 is far behind it on Hindi (WER 27.9 on Kathbath, 33.2 on FLEURS-hi, Phase 3, against 11.4 on Kathbath). Large-v3 q5_0 stays Hindi's night rank in the manifest only as the fallback of a machine without a measured Hinglish model.
* The committed `models/manifest.json` is generated from the git-ignored `models/manifest.local.json` (including its night and live ranks); it still ranks turbo first for Hindi live, which section 3 does not support (harmless while `validated_roles` is empty).

## 8. Sources and licences

Data sets (`eval/data/SOURCES.md`, git-ignored): FLEURS (CC BY 4.0), Kathbath hi and ur (CC0 packaging; AI4Bharat, IndicSUPERB, terms
accepted by the developer), Svarah (AI4Bharat; CC BY 4.0), CoSHE-Eval (CC BY-NC 4.0; evaluation only). Models
(`models/manifest.json`): whisper small, medium, large-v3 and large-v3-turbo q5_0 from `ggerganov/whisper.cpp` (MIT); Silero VAD
v6.2.0 (MIT); the Hinglish fine-tune of large-v3 (Apache-2.0, notice in `models/NOTICE-hinglish.txt`). Evaluation scripts:
`eval/run.py`, `eval/item8_report.py`, `eval/item10_report.py`, `scripts/gates/night_q8_report.py`; settings in `eval/roles.toml`;
the run list in `eval/runs.toml`.
