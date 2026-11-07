# Phase 3: System check, capability profile and Start Session dialog

## Context
Engines and the model registry are merged (Phase 2). The room machines are undecided and will differ, so every machine must measure what it can do before an operator relies on it, and the app must choose devices, models and thread counts from those measurements (decisions D2, D4, D5 and D13; AGENTS.md non-negotiables 3 and 7). Live text is off by default everywhere and is chosen per recording. Read `docs/PLAN.md` sections 2, 3 and 4.

## Goal
A system check that probes the hardware, tries each engine build safely, benchmarks the candidate models on each device configuration exactly as they would be used, checks that the recorder stays safe, applies the PLAN section 3 rules, saves `capability.json` and explains the verdict on the System screen; and a Start Session dialog that reads the profile before every recording.

## Steps
1. Probe in Rust (under 10 seconds): sysinfo (CPU brand, physical and logical cores counted from the process affinity mask, RAM total and available); CPU features; GPUs (nvml-wrapper for NVIDIA name, VRAM and driver when `nvml.dll` loads; the DXGI adapter list for every GPU, marking integrated ones and their shared memory); free space on the recording and backup roots; power source (`GetSystemPowerStatus`); audio inputs (cpal). When `WB_EMULATE` is set (`docs/LEGION-FIRST.md` section 2), report 16 GB total, cap available memory at the job limit, list only the visible GPUs, mark `capability.json` as emulated and show an Emulated badge on the System screen in development builds.
2. Engine sanity: run each bundled variant's `info` with a 20-second timeout. A crash, timeout or non-zero exit marks that variant or device unusable, with the reason; the app never crashes.
3. Device configurations to measure: CPU only; GPU only (each usable GPU); CPU and GPU together (a CPU engine and a GPU engine on separate work units, threads split so the GPU engine keeps one or two); and GPU encoder with CPU decoder if the experimental `openvino` build is present.
4. Benchmark plan from the manifest: candidates whose size fits RAM or VRAM (shared GPU memory counted) with a 30% margin, for the languages in `docs/programme.json`. Quick check (first launch, under 8 minutes on the low-baseline laptop): each language's near-live candidate on the CPU (greedy, utterance-length chunks, half the logical threads), each language's rolling candidates (beam 5 on 60 seconds) on every usable configuration, the code-switched clip with the English + Hindi routing, and the GPU live candidate if a discrete GPU build works. Full check (System screen button): every candidate, the night candidates, and two thread counts on hybrid CPUs.
5. Recorder safety run: record from the default input for 2 minutes while the chosen configuration runs; any overrun rules that configuration out.
6. Bundled clips: 60 seconds per language from the FLEURS test split (CC BY 4.0) plus one code-switched English–Hindi clip recorded for the project (consent noted), with attribution in `THIRD_PARTY_LICENCES.md`, in `resources/bench/`.
7. Decision rules exactly as the PLAN section 3 table: post-processing configuration; tier; per language, near-live or live availability; rolling and night models; thread counts; memory headroom; and the estimated time to a complete transcript after a 90-minute talk (PLAN section 4 formula).
8. `capability.json` in the app data folder: schema version; fingerprint (CPU, cores, RAM, GPUs and drivers, app version, manifest hash); each measurement with machine, engine build, device configuration, threads and method; the decisions; timestamp; overrides. A changed fingerprint or manifest shows a banner and re-runs the quick check; the operator can re-run at any time.
9. System screen per `design/mockups/system-check.html`: Hardware and Speed test tables (with the device configuration per row), a Verdict card in plain words, a "Ready after a 90-minute talk" card, and Overrides (Process while recording, Pause on battery power, Allow live captions anyway). An override beyond the verdict needs a confirmation Dialog and is logged in the profile.
10. Start Session dialog per `design/mockups/start-session.html`, shown before every recording: expected languages (English and Hindi ticked, Urdu optional); mode, with Record + post-process selected by default on every machine and Live + post-process disabled with its reason unless the profile clears every ticked language; where post-processing will run and the estimated ready time. The choice is saved in `session.json`.

## Verification
- Unit tests for the decision rules with fixture profiles: tier G laptop, tier C+ laptop with a strong integrated GPU, tier C low-baseline laptop, tier R old PC, a machine whose `avx512` build crashes, and a machine whose integrated GPU is slower than its CPU (hybrid must not be chosen).
- On the low-baseline laptop (until it arrives, under `emu-low`, PROVISIONAL, with the 25% speed headroom rule): the quick check finishes in under 8 minutes (log), the verdict follows the rules from the measured numbers, and the hybrid configuration is chosen only if it measured faster (attach `capability.json`).
- On the mid laptop (until it arrives, under `emu-mid`, PROVISIONAL) and the Legion (`native`): the same, attached; the Legion is tier G with the `cuda` build.
- The Start Session dialog defaults to Record + post-process on all three machines; live is selectable only on machines and language sets the profile clears (Playwright with fixture profiles, plus screenshots).
- A test hook that changes the fingerprint triggers the banner and a re-run.
- Screenshots of System and the Start Session dialog beside `system-check.png` and `start-session.png`.
- `design-consistency-reviewer`, `asr-quality-auditor` (rules, measurement method, device configurations) and `offline-auditor` report PASS.

## If Blocked
- NVML is missing: fall back to DXGI and the `cuda` engine's `info`.
- An integrated GPU's shared memory can't be read: assume half of system RAM is available to it, mark the value as assumed, and keep the 3 GB free rule.
- The quick check runs long on the low-baseline laptop: test one language per role first, finish the rest in the background, and mark the profile "partial" until it completes.
