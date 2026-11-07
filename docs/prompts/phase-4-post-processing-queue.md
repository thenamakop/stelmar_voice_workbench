# Phase 4: Post-processing queue, hybrid scheduler and night pass

## Context
The recorder (Phase 1), engines (Phase 2) and system check with the Start Session dialog (Phase 3) are merged. Post-processing is the main transcription path on every room machine (decisions D4, D6, D12 and D13), and speakers switch between English and Hindi within a talk (D8). Read `docs/PLAN.md` sections 4, 5, 6 and 8.

## Goal
A queue that transcribes recordings on the same machine while recording continues: a rolling pass that follows each session part by part, an optional night pass with a more accurate model, work units that can run on the CPU and GPU at the same time, per-chunk language handling, resumable jobs, safe use of CPU, GPU and memory, a Queue screen, and import of session folders from other machines.

## Steps
1. Store: SQLite tables for jobs (id, session, kind rolling/night/translate/import, models by language, languages, state, `cursor_ms`, created, started, finished, error), work units (job, start_ms, end_ms, device, state, attempts) and segments with revision (`post-1` rolling, `post-2` night), `lang`, `lang_prob`, `mixed`, `device` and a unique key (session, revision, start_ms); JSONL per revision, appended and fsynced by the app before any UI event.
2. Work units: cut each job into units of about 5 minutes that start and end at VAD pauses. Units run on whichever engine is free; the app buffers finished units and commits their segments in audio order, advancing `cursor_ms` only over a continuous committed run.
3. Rolling pass: created when a session starts. On each `part_closed` event, add units up to 2 seconds before the end of the last closed part; at Stop, add the rest. Each unit carries the session's expected languages and the operator's lock log for its time range; the glossary comes from the session.
4. Hybrid configuration: when `capability.json` chose CPU + GPU, run a GPU engine (`vulkan` or `cuda`) and a CPU engine together, with the thread split from the profile. Otherwise run the single chosen engine. If the GPU engine fails, its units return to the queue and run on the CPU.
5. Night pass (D6): after the day's last session, for each language whose night model ranks above its rolling model and whose estimated time fits before 08:00, re-transcribe that day's sessions into `post-2`; `post-2` becomes the default revision once complete.
6. Scheduler: priority recording > live > rolling > night > translation; engines at below-normal priority; pause when free RAM (shared GPU memory counted) is under 2 GB, on battery (unless overridden), on any recorder overrun, and during live sessions on CPU-only machines; resume automatically and log every pause with its reason.
7. Resume and integrity: after a crash or restart, jobs continue from `cursor_ms` and re-run unfinished units; the unique key prevents duplicates; a unit that fails three times on one device moves to the other device, and a job whose unit fails everywhere is parked with its error shown.
8. Estimates: from the measured throughput of the configuration and the remaining audio, per job and per day ("All transcripts ready by 19:40").
9. Queue screen: one row per job (session, models, devices, threads, progress bar, estimate, state Badge) with Pause, Resume, Move Up, Change Model (Dialog) and Open Session, plus a Scheduler card with the current configuration and resource use. Record screen in record mode: the rolling transcript (language Badge per segment, mixed segments marked) and parts strip per `design/mockups/record-only.html`.
10. Import: register a session folder copied from another machine (a record-only machine or another room) and queue it here.

## Verification
- Tests: unit cutting at VAD pauses; in-order commit with units finishing out of order on two engines; cursor logic across part boundaries (no word cut at a boundary, no duplicates); resume after kills at random points (property test); priority and pause rules; a lock-log change inside a unit applied at the right chunk.
- Day simulation on the low-baseline laptop and on the mid laptop: play the four transcribed Day 1 sessions with the programme's timings (6 hours of audio between 10:00 and 17:30, more than any single room records), including at least one session that switches between English and Hindi, through the virtual cable in Record + post-process mode, then let the night pass run. Zero recorder overruns; each rolling transcript completes within 10% of the estimate for the measured configuration; the hybrid configuration runs only where the profile chose it; the night pass finishes before 08:00 or is correctly skipped. Attach logs and a timeline chart per laptop.
- Kill the CPU engine and the GPU engine 5 times each during a simulation: no segment is lost or repeated, and the transcript stays in audio order.
- Screenshots of Queue and Record (record mode) beside the mockups.
- `asr-quality-auditor` (scheduler, units, cursor, devices, languages), `design-consistency-reviewer` and `offline-auditor` report PASS.

## If Blocked
- The rolling model can't keep up on the low-baseline laptop: use the next faster validated model for the rolling pass and leave the better one to the night pass; report the numbers.
- CPU + GPU together is slower than either alone on a laptop (shared power budget): keep the faster single device for that machine; the profile already records why.
- Recorder overruns appear under load: reduce batch threads by one, or move work to the GPU, and repeat the load test; report.
