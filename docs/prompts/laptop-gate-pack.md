# Laptop gate pack

## Context
Phases 0 to 3 have been merged with their laptop gates marked PROVISIONAL or DEFERRED in `docs/GATES.md`, because the test laptops were not available (`docs/LEGION-FIRST.md`). When the laptops arrive they must re-run those gates quickly, without development tools, and send the results back. Read `docs/LEGION-FIRST.md` sections 3 and 7 and `docs/GATES.md`.

## Goal
A `gate-pack/` folder, built on the Legion, that a person copies to a laptop by USB drive and runs with one command. It runs every PROVISIONAL and DEFERRED gate of the merged phases on that laptop, collects logs and a summary, and zips the results. A Legion script then reads the results into `docs/GATES.md`.

## Steps
1. `scripts/build-gate-pack.ps1` builds release engines (`GGML_NATIVE=OFF`), the app's release build, and the test binaries the gates need, and copies them into `gate-pack/` (git-ignored) with the bench clips, the models the gates use (with `manifest.json`) and a `README.txt` in plain words.
2. `gate-pack/run-gates.ps1` (run as administrator): records the machine fingerprint (CPU, cores, RAM, GPUs and driver versions, Windows build); then runs each pending gate from `docs/GATES.md` with the same method its phase prompt describes: the Phase 1 soak (2 hours, through VB-CABLE, with a prompt to install it if missing) and load and kill tests; the Phase 2 `info` checks for every engine variant, the Vulkan check on the integrated GPU, the transcription checks and the speed table; the Phase 3 quick check with its time and the resulting `capability.json`. Long gates run last, and the script can resume after a restart. The 30-minute load gate runs twice on each laptop: once within one hour of a cold boot (the script asks for a restart and starts the gate when the person logs in, with the uptime recorded) and once on a settled machine that has been up for more than 3 hours and idle. LatencyMon, RTSS and MSI Afterburner must be closed for both (the gate refuses to start otherwise). The cold-cache hashing gate (R5) runs at least five times after a restart and five times with the models already hashed on each laptop, and the recorder gates use the hardware audio interface where one is available, because on VB-CABLE the load and hashing gates of 13 Nov 2026 each used their whole upstream-break allowance (3 of 3 and 1 of 1) and two earlier hashing runs had shared upstream breaks while the models were being hashed; the results report the shared-break count next to each verdict, and a break that is only in the app is a recorder failure. The kill gate also prints the capture margin of its final run (`docs/GATES.md`, 13 Nov 2026 rows).
   The Phase 3 quick check on the real low laptop also reports the per-cell times, **including the graphics-alone run of the hybrid test** (`hybrid_gpu_alone`, up to a 300 s timeout, outside the quick check's remaining-time cap), against the 480 s bar (GATES, 14 Nov 2026, "graphics-alone run outside the quick-check time cap"). If the quick check is over 480 s, fix the cap after the laptops are measured; do not change it before.
3. Output: `gate-results-<machine>.zip` with every log, the speed table, `capability.json` and `summary.json` (one entry per gate: id, status PASS or FAIL, the measured numbers and the threshold).
4. `scripts/ingest-gate-results.ps1` on the Legion: reads a results zip, writes the logs to `docs/evidence/<machine>/`, updates the matching rows in `docs/GATES.md` (PASS or FAIL, real machine name, date) and fills in the Machines table.
5. Dry run: copy `gate-pack/` to a folder outside the repository and run it under `wb-emulate --profile emu-low` with development tools removed from `PATH`.

## Verification
- The dry run under `emu-low` completes (the 2-hour soak can be shortened to 10 minutes with a `-Quick` switch for the dry run only) and produces a results zip.
- `ingest-gate-results.ps1` turns the dry-run zip into ledger rows marked as emulated, not as a laptop PASS.
- `gate-pack/` runs with no network and contains no development tools, source code or private audio.
- `offline-auditor` reports PASS.

## If Blocked
- A gate can't run without a development tool: build what it needs into the pack, or mark it as Legion-only in `docs/GATES.md` with the reason.
- The pack is too big for a USB drive: leave out models a gate doesn't use and report the size.
