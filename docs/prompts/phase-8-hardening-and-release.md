# Phase 8: Hardening, offline proof and release

## Context
All features are merged. Prove the app is offline and stable for a full conference day on the kind of laptops the rooms may get, then ship installers, model packs and the operator documents. The room machines are still undecided, so the release must work on any 16 GB laptop with AVX2 and pick its own configuration. Read `docs/PLAN.md` sections 10, 11, 14 and 15.

## Goal
A Windows installer that works on a clean 16 GB laptop with no internet; CPU and GPU model packs with checksums; written evidence of offline operation, a full-day simulation on both test laptops and fault handling; an operator guide and runbook.

## Steps
1. Offline proof script (PowerShell, run as administrator) with two runs of a scripted 30-minute session played from a file through the virtual cable, with the hybrid configuration active where the profile chose it. Watched run, network connected: turn on Filtering Platform connection auditing (`auditpol`, events 5156 and 5157) and poll `Get-NetTCPConnection` and `Get-NetUDPEndpoint` every 5 seconds for the app's process tree (the app, its `wb-engine` processes and the `msedgewebview2.exe` processes it starts, found by parent process ID); any connection or endpoint beyond loopback fails the run. Event-day run: outbound firewall block rule for the app and network adapters disabled; the session must record and transcribe normally. Save both logs.
2. Day simulation on the low-baseline laptop and on the mid laptop: the four transcribed Day 1 sessions with the programme's timings (6 hours of audio between 10:00 and 17:30, more than any single room records), with English–Hindi switching in at least two sessions, in Record + post-process mode, then the night pass. Every minute, log memory (including shared GPU memory), CPU and GPU load, recorder overruns, queue lag, and estimated against actual finish times. Targets: zero recorder gaps; memory growth under 300 MB across the day; rolling transcripts within 10% of their estimates; the night pass finished by 08:00 or correctly skipped.
3. Fault drills, each with expected and observed behaviour: unplug the audio interface; remove the mirror drive; fill the disk to the warning threshold; force-kill the app; force-kill each batch engine mid-unit; make the GPU engine fail (disable the integrated GPU in Device Manager) and check that its units move to the CPU; push free RAM under 2 GB (the queue pauses, the recorder doesn't); unplug the charger mid-session; try to sleep the machine during a session.
4. Installer: `pnpm tauri build` producing NSIS and MSI with `bundle.windows.webviewInstallMode` set to `offlineInstaller`, with all engine variants as sidecars (the `openvino` variant only if Phase 4 showed it beats Vulkan on Intel laptops). Bundle the CUDA runtime DLLs only where NVIDIA's licence allows redistribution; otherwise the `cuda` variant becomes an optional, documented setup step.
5. Model packs: a CPU pack (the validated small and medium-class models and turbo) and a GPU pack (adds large-v3 and the code-switching model if validated), each a folder or zip with `manifest.json`; an "Install Model Pack" action that copies the files, verifies hashes and re-runs the system check.
6. Documents: `docs/OPERATOR_GUIDE.md` with screenshots (System check, Start Session dialog, Record in both modes, Queue, Review), `docs/RUNBOOK.md` from PLAN section 15, `docs/BUILD.md`, a setup-week checklist for unknown room machines (drivers, power plan, system check), and `THIRD_PARTY_LICENCES.md` (Whisper MIT, whisper.cpp MIT, Silero VAD MIT, each model's licence from the manifest, OpenVINO Apache 2.0 if shipped, FLEURS test clips CC BY 4.0, fonts OFL, crates and npm packages).

## Verification
- The watched-run log shows no connection, endpoint or audit event beyond loopback from the app's process tree for the whole 30 minutes; the event-day run completes normally.
- The day simulation reports meet every target on both laptops; the fault-drill table is complete.
- On a clean Windows 11, 16 GB laptop with no internet: install the app and the CPU pack; first launch runs the system check; the Start Session dialog defaults to Record + post-process; record a session that switches between English and Hindi; the transcript is produced; export it.
- A dry run in a real room with the mixer feed and a speaker who switches languages (a person signs off).
- `offline-auditor`, `asr-quality-auditor` and `design-consistency-reviewer` report PASS.

## If Blocked
- CUDA redistribution terms are unclear: make the `cuda` variant an optional installation step and document it.
- The WebView2 runtime itself opens a connection: identify the endpoint and turn the feature off through the window's `additionalBrowserArgs`, keeping Tauri's default `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` (setting the option replaces it); keep the outbound firewall rule on event day.
- The day simulation shows the queue falling behind on the low-baseline laptop: move that language's rolling model down one size, keep the better model for the night pass, and record the change in `eval/REPORT.md`.
