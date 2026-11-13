# Phase 2b: recorder hardening (`phase-2b-recorder-hardening`)

Phase 2 is merged (PR #4). The recorder must be final before the laptop gate pack is built on Mon 16 Nov 2026. This phase closes R1, R4 and R5 of `docs/GATES.md` (rows dated 12 Nov 2026), checks the R3 count, re-runs the recorder gates once on the final recorder, and opens a pull request. It touches `crates/wb-recorder`, `crates/wb-models` and their tests, plus the gate scripts; the one edit outside them is the thread setup in `src-tauri/src/engines.rs` that R4 needs (approved by the owner).

## Steps

1. Plan first; no recorder file changes before the owner approves it.
2. **R1.** In the recorder thread, write the WAV block first, then run the repeat and zero-gap detectors on it. Wrap each detector in `catch_unwind`: a panic disables that detector for the rest of the session and records one event; it never stops the recording. Give the detectors a time budget per block (block duration / 20, at least 2 ms, checked every 10 ms of audio): over budget means skip and count it. Tests: a detector that panics (recording continues, every sample written) and a detector that is slow (budget enforced, no ring overrun).
3. **R4.** Run the model-hashing thread with `THREAD_MODE_BACKGROUND_BEGIN` (CPU and I/O priority) and end it properly. Unsafe code only in a `win.rs` listed in `docs/UNSAFE.md`. Test that the mode is set. The log line for a hashed file carries the file name only, never a local path.
4. **R5.** A gate case that starts a recording while the model hash cache is cold and hashing runs over the real model files (hash cache cleared first). Pass: zero overruns, zero dropped samples, fidelity equal to the reference, under `emu-low`. The evidence records whether the run started after a reboot; without a reboot R5 is "warm cache, not conclusive" and a hashing-only re-run after the next reboot stays open.
5. **R3.** Check the "30 of 240" synthetic-loop figure against `repeated-block-validation.txt` and correct the GATES row. Measure the `repeated_block` detection rate on real speech: repeats spliced into FLEURS clips from `eval/data` (never committed), lag 400 to 1,200 frames, length 256 to 1,200 frames, and report the rate with its method in the R3 row.
6. Reviewers (`asr-quality-auditor`, `offline-auditor`) on the tree before the gate build, so a change they cause cannot invalidate the night run.
7. Night run on the final recorder (gate build last, nothing else building), under `emu-low`, order `hashing, panic, webview, kill, load, soak`, with the live detector table and the measured longest detector time per block.
8. Full check suite, attribution scan, push, open the PR, report the PR number and CI. Do not merge.
9. Keep a handoff note at `D:\STELMAR-Gate\handoff-phase2b.md`, updated as the last step of every turn.

## Verification

- Unit tests for a panicking and a slow detector pass, with every sample written.
- The night run gives PASS (PROVISIONAL) for all gates including R5, with the live detector table and no false alarms outside lead-in.

## If blocked

- If R1 needs a change to the capture callback (not just the recorder thread), stop and ask.
- If any gate FAILs or is INVALID, stop and report; do not re-run without asking.
