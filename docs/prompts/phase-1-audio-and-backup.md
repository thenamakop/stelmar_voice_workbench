# Phase 1: Audio capture and crash-safe recording

## Context
Phase 0 is merged: the STELMAR-styled shell runs offline. Recording is the foundation: every transcript is made from these files, and nothing may disturb them (AGENTS.md, non-negotiable 2). Read `docs/PLAN.md` section 8. From now on, recording gates are measured on the low-baseline laptop (the weakest 16 GB laptop available), because the room machines are undecided and will differ. The laptops are not available before 17 November 2026, so this phase also builds the Legion emulation launcher, and laptop gates run under `emu-low` as PROVISIONAL until then (`docs/LEGION-FIRST.md`).

## Goal
Input device selection, a live level meter, crash-safe 48 kHz WAV recording in 10-minute parts with a parts index, a live mirror to a second drive, session folders and crash recovery, all in Rust, with a raised-priority recorder that keeps working while the CPU and GPU are fully loaded.

## Steps
1. Crates: cpal, ringbuf, hound, serde and serde_json, time, thiserror, anyhow, tracing, `windows` (MMCSS, power requests). The cpal callback only copies samples into a lock-free ring buffer holding at least 10 seconds and counts overruns.
2. Devices: list inputs and supported configurations; prefer 48 kHz; remember the last device; if the device disappears, show a banner, retry every 2 seconds and start a new part on reconnect.
3. Recorder thread registered with MMCSS ("Pro Audio" through `AvSetMmThreadCharacteristicsW`; fall back to `THREAD_PRIORITY_HIGHEST` and log it): 16-bit WAV at the device rate, channels as captured; 10-minute parts `audio/part-0001.wav`; `WavWriter::flush()` every second; fsync every 5 seconds; never more than 2 seconds unwritten.
4. Parts index: when a part closes, append `{part, start_ms, duration_ms, sample_rate, channels, sha256}` to `parts.json` (written atomically) and emit a `part_closed` event. Phase 4's post-processing cursor and Phase 6's player follow this file.
5. Mirror thread: copy closed parts and keep the open part in sync under the backup root with the same relative path; show lag; warn if the drive is missing or more than 30 seconds behind.
6. Session folders and `session.json` exactly as in PLAN section 8, with `opened_at`, `closed_at`, expected languages, mode and a language log; warn at 20 GB free and refuse new sessions below 5 GB; keep the machine awake during a session (`SetThreadExecutionState`).
7. Crash recovery at start-up: sessions without `closed_at` get WAV headers repaired from file length and `parts.json` rebuilt from the files; the operator chooses Resume or Close.
8. Tauri commands `list_inputs`, `start_session`, `pause`, `resume`, `stop_session`; channel events `level` (RMS and peak in dBFS at 20 Hz), `recorder_status` (including the overrun count), `mirror_status`, `part_closed`.
9. Record screen recording state per `design/mockups/record-only.html`: recording Badge, clock in IBM Plex Mono, Pause and Stop Session buttons, Input card with a peak-hold meter, Backup card, and the parts strip (Recording and Waiting states until Phase 4 adds transcription).
10. Legion emulation (development only, never bundled): `crates/wb-emulate` per `docs/LEGION-FIRST.md` section 2. It creates a Job Object with `JOB_OBJECT_LIMIT_AFFINITY` and `JOB_OBJECT_LIMIT_JOB_MEMORY`, picks cores by efficiency class from `GetLogicalProcessorInformationEx`, starts the given command inside the job and sets `WB_EMULATE`, `CUDA_VISIBLE_DEVICES=-1` and `GGML_VK_VISIBLE_DEVICES`. Profiles `emu-low`, `emu-mid` and `native`. Add `scripts/emulate.ps1` as a thin wrapper and record every laptop gate of this phase in `docs/GATES.md`.

## Verification
- Unit tests: writer round trip with a synthetic sine (samples identical); header repair on a truncated file; `parts.json` rebuilt correctly from a folder of parts.
- Soak on the low-baseline laptop (until it arrives, under `emu-low`, PROVISIONAL): 2 hours from a virtual audio cable (for example VB-CABLE) playing a known file; recorded duration within 1 second of the source; no silent gaps; recorder CPU under 5%.
- Load test on the low-baseline laptop (until it arrives, under `emu-low` with the stress script inside the same job, PROVISIONAL): 30 minutes of recording while a stress script keeps every logical CPU busy at normal priority and a GPU load runs on the integrated GPU; zero ring-buffer overruns and no gaps (log attached).
- Kill test, 5 times: force-kill mid-recording (`taskkill /F`); every part plays in a standard player; at most 1 second lost each time; restart offers Resume and `parts.json` is correct.
- After Stop, SHA-256 of mirror files equals the originals.
- Emulator: under `emu-low` a child process sees exactly 4 logical processors, all efficiency cores (affinity mask logged), cannot commit more than 11 GB, and `nvidia-smi` shows no process of the job; under `emu-mid` it sees 12.
- `docs/GATES.md` has a row for every machine-named item of this phase.
- `design-consistency-reviewer` and `offline-auditor` report PASS.

## If Blocked
- The device won't open at 48 kHz: record at its native rate; resampling for speech happens in the engines.
- WASAPI exclusive mode misbehaves: use shared mode.
- Overruns appear under load: raise the ring buffer to 30 seconds and check the recorder thread's priority before anything else; report what changed.
