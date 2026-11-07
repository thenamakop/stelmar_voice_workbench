# Laptop gate pack

## Context
Phases 0 to 3 have been merged with their laptop gates marked PROVISIONAL or DEFERRED in `docs/GATES.md`, because the test laptops were not available (`docs/LEGION-FIRST.md`). When the laptops arrive they must re-run those gates quickly, without development tools, and send the results back. Read `docs/LEGION-FIRST.md` sections 3 and 7 and `docs/GATES.md`.

## Goal
A `gate-pack/` folder, built on the Legion, that a person copies to a laptop by USB drive and runs with one command. It runs every PROVISIONAL and DEFERRED gate of the merged phases on that laptop, collects logs and a summary, and zips the results. A Legion script then reads the results into `docs/GATES.md`.

## Steps
1. `scripts/build-gate-pack.ps1` builds release engines (`GGML_NATIVE=OFF`), the app's release build, and the test binaries the gates need, and copies them into `gate-pack/` (git-ignored) with the bench clips, the models the gates use (with `manifest.json`) and a `README.txt` in plain words.
2. `gate-pack/run-gates.ps1` (run as administrator): records the machine fingerprint (CPU, cores, RAM, GPUs and driver versions, Windows build); then runs each pending gate from `docs/GATES.md` with the same method its phase prompt describes: the Phase 1 soak (2 hours, through VB-CABLE, with a prompt to install it if missing) and load and kill tests; the Phase 2 `info` checks for every engine variant, the Vulkan check on the integrated GPU, the transcription checks and the speed table; the Phase 3 quick check with its time and the resulting `capability.json`. Long gates run last, and the script can resume after a restart.
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
