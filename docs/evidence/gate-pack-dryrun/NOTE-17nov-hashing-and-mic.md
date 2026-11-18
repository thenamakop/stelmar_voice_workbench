# Dry run of 17 Nov 2026: reading of the two hashing FAILs and of the mic run

Written by the coding agent (Claude Code) on 17 Nov 2026, not by a script. The evidence files next to this note are written by `scripts/ingest-gate-results.ps1`; this note is a reading of them.

Machine: the Legion under `emu-low`, `-Quick`. Pack commit 38699fd (zip created 17 Nov 2026 14:29). Nothing here is a laptop result.

## The two FAILs of part B

| Gate | Ran (Legion clock) | Rule that failed | Measured | Bar | Class |
| --- | --- | --- | --- | --- | --- |
| hashing-1 | 10:37:42 to 10:43:21, within 5 min of the 10:33:38 boot | "model files were hashed while the recording ran ... at least 10 s of hashing inside the recording" | 6 files, 4.76 GB at 962 MB/s: 4.5 s of hashing inside the recording | 10 s | (a) pack bug: the bar came from the Legion's 9.66 GB models folder (11 to 14 s) and was never reachable by the pack's 4.76 GB set on a fast disk |
| hashing-2-10, run 2 | 12:07:24 to 12:13:16, 95 min after boot | the same rule | 4.76 GB at 914 MB/s: 4.8 s | 10 s | (a) the same bug. The run was also not cold (rate above the cold limit), which the verdict already reports as INFO, never FAIL (class d handled correctly); only the 10 s rule made it FAIL |

Every other check of both runs passed: zero overruns, zero dropped samples, fidelity equal to the reference captures.

Neither gate ran in the resumed window after the restart (only load-cold did, 13:46:18 to 13:53:24, after the 13:44:49 boot), so this is not a runner bug.

## Change (commit aeb32fb)

`Test-WbHashWindow` (scripts/lib/Fidelity.psm1): the window is long enough when at least 10 s of hashing ran inside the recording, or when the whole model set (every hashed file overlapped the recording, at least 4 GB) was hashed while it ran. A FAIL headline now lists the failed checks. Tests: `Fidelity.Tests.ps1`, `RecorderGates.Tests.ps1`. The rows below keep the FAIL of the old rule; they are replaced when the gates are run again with the new pack.

## The mic run

The mic-run row of the part B zip is WAITING only because mic-run ran in a separate work folder (`D:\STELMAR-Gate\mic-work`, pack 65e05bf, zip created 17 Nov 2026 04:59, label dryrun-mic2) after the disk-space fix: PASS, `-Quick` (15 min), on the Legion's own microphone, not emulated. It is filed under `legion-native-mic` as a dry run, never as a laptop. The 17 WAITING rows of that zip only mean "not run in that work folder".

## Re-run of the two hashing gates (pack 6b68837, same day)

Written by the coding agent (Claude Code), 17 Nov 2026, not by a script. Evidence: `docs/evidence/gate-pack-dryrun/emu-low-hash-rerun/` (its own folder, so the files of the earlier run above stay as they were).

| Gate | Ran (Legion clock) | Result | Numbers |
| --- | --- | --- | --- |
| hashing-1 | 17:29:02 to about 17:35, 1.2 min after the 17:27 restart | PASS (PROVISIONAL: emulated), 5.8 min | the new window rule holds; zero overruns and no dropped samples |
| hashing-2-10 | 17:35:27 to about 17:59, 24 min | INFO "not cold": 0 of 1 purged runs counted as cold, warm-cache rate 1,410 MB/s | 4 recorder runs (run 2 purged, run 3 warm, then 2 more purged runs because run 2 did not count as cold); every run kept the recorder clean |

The 24 minutes are 4 runs of about 6 minutes (4-minute recording plus about 2 minutes of set-up), not the 12 minutes the pack had printed. The extra purged runs are the rule "fewer cold runs than needed: up to 2 more". The quick estimate is now 24 min for hashing-2-10 and 6 min for hashing-1; at full length hashing-2-10 is 132 min (9 runs of about 12 min = 108, plus up to 2 extra). The Legion reads faster than the cold-disk limit, so INFO "not cold" is the expected result here; on a laptop with a slower disk the purged runs should count as cold.
