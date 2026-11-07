# Legion-first plan: 8 to 17 November 2026

The two test laptops are not available before 17 November 2026. Until then all work happens on the Legion (Core i9-14900HX: 8 performance cores with Hyper-Threading and 16 efficiency cores; 32 GB RAM; Intel UHD integrated GPU; RTX 4070 Laptop GPU). This file says what gets built before the laptops arrive, how laptop gates are stood in for on the Legion, and what happens on the day they arrive.

## 1. Target for 17 November

| Track | By 17 November | Stretch |
| --- | --- | --- |
| A: main branch, one Claude Code session | Phases 0, 1, 2 and 3 merged, with laptop gates passed as PROVISIONAL under emulation | Phase 4 started |
| B: second Claude Code session in a git worktree | Phase 5a (evaluation groundwork) merged | Hindi and Urdu fine-tunes converted and quantised |
| Gate pack | `gate-pack/` built and dry-run under emulation, ready to copy to the laptops | |
| People | Audio interface and SSD in hand; laptop models known | |

Recording and transcribing the local test audio is scheduled for later, once most of the development work is done (after Phase 4); see `docs/TEST-AUDIO.md`.

## 2. Legion emulation

The Legion stands in for the laptops by running the app and its engines inside a Windows Job Object that limits CPU cores and memory and hides the NVIDIA GPU. Phase 1 builds this as `crates/wb-emulate`, a development-only launcher that is never bundled.

| Profile | Stands in for | CPU (from the process affinity mask) | GPU | Memory |
| --- | --- | --- | --- | --- |
| `emu-low` | Low-baseline laptop (4 cores, basic integrated graphics) | 4 efficiency cores, 4 threads | Intel UHD only | Job limit 11 GB, reported as a 16 GB machine |
| `emu-mid` | Mid laptop (for example Core i5-1235U: 2 performance and 8 efficiency cores) | 2 performance cores with Hyper-Threading plus 8 efficiency cores, 12 threads | Intel UHD only | Job limit 11 GB, reported as a 16 GB machine |
| `native` | The Legion itself | All 32 threads | RTX 4070 and Intel UHD | 32 GB |

How it works:

1. `wb-emulate --profile emu-low -- <command>` creates a Job Object with `JOB_OBJECT_LIMIT_AFFINITY` and `JOB_OBJECT_LIMIT_JOB_MEMORY`, starts the command inside it (child processes such as engines inherit the job), and sets `WB_EMULATE=<profile>`, `CUDA_VISIBLE_DEVICES=-1` and `GGML_VK_VISIBLE_DEVICES=<index of the Intel GPU>`.
2. Cores are chosen by efficiency class from `GetLogicalProcessorInformationEx`, never by assumed numbering.
3. When `WB_EMULATE` is set, the system check reports 16 GB total, caps available memory at the job limit, counts cores from the affinity mask, lists only the visible GPUs, marks `capability.json` as `"emulated": "<profile>"` and shows an Emulated badge on the System screen (development builds only). An emulated profile is never copied to another machine.

Before the first run: set the Legion's GPU mode to Hybrid in Lenovo Vantage (or the BIOS) so the Intel GPU is visible, and check that Device Manager lists both GPUs.

### What emulation can and cannot show

| Emulation is | Because | Rule |
| --- | --- | --- |
| Optimistic for CPU speed | Efficiency cores on the Legion run at higher sustained clocks, on better cooling, than a 15 to 28 W laptop | Speed gates under emulation need 25% headroom: RTF ≤ 0.75 where the gate says 1.0, ≤ 0.26 where it says 0.35 |
| Pessimistic for integrated-GPU speed | The 14900HX's UHD graphics is weaker than Iris Xe, Arc or Radeon 680M/780M | Hybrid gains measured here are a floor, not an estimate |
| Exact for memory rules | The job limit enforces the budget | Same thresholds (3 GB free to start, pause under 2 GB) |
| Good for recorder safety | Overruns depend mostly on priority and core contention, which the affinity mask reproduces | Same gate: zero overruns |
| Blind to other drivers and thermals | Only one integrated GPU and one chassis are available | These gates stay DEFERRED until the laptops arrive |

## 3. Gate statuses

Every Verification item that names a laptop is recorded in `docs/GATES.md` with one of these statuses:

| Status | Meaning | Lets the next phase start? |
| --- | --- | --- |
| PASS | Passed on the named machine | Yes |
| PROVISIONAL | Passed under `emu-low` or `emu-mid` on the Legion, with the headroom rule | Yes, until the laptops arrive |
| DEFERRED | Cannot be emulated (other GPU drivers, 2-hour thermal behaviour on a thin laptop, real room hardware) | Yes, only for items listed as DEFERRED in this file |
| FAIL | Failed anywhere | No |

Only these items may be DEFERRED; everything else that names a laptop runs under emulation:

1. Phase 1: thermal behaviour during the 2-hour soak on a thin laptop (the soak itself runs PROVISIONAL under `emu-low`).
2. Phase 2: portable engine builds running on a CPU other than the build machine's, and Vulkan on the laptops' own integrated GPUs and drivers.
3. Phase 3: the verdicts and tiers chosen on the real laptops, and the quick-check time on the real low-baseline laptop.

When the laptops arrive, every PROVISIONAL and DEFERRED item is re-run on them with the gate pack. A failure there reopens its phase, and no further phase is merged until it is fixed.

## 4. Day by day

| Day | Track A (main session) | Track B (worktree session) | People |
| --- | --- | --- | --- |
| Sun 8 Nov | Legion setup (section 5); new repo; `/run-phase 0` | | Run `scripts/setup-legion.ps1` |
| Mon 9 Nov | Phase 0 finished, reviewed, merged | | |
| Tue 10 Nov | `/run-phase 1`, including `wb-emulate` | | |
| Wed 11 Nov | Phase 1: 2-hour soak under `emu-low` overnight | Start Phase 5a; model and dataset downloads overnight | |
| Thu 12 Nov | Phase 1 merged; `/run-phase 2` (CUDA and Vulkan builds run in the background) | Phase 5a: data, normalisers, tests | Buy or borrow the USB audio interface and external SSD |
| Fri 13 Nov | Phase 2: engines, Vulkan on the Intel GPU, speed tables under all three profiles | Phase 5a: conversion and quantisation scripts | |
| Sat 14 Nov | Phase 2 merged; `/run-phase 3` | Phase 5a merged | Ask for the make and model of the two laptops |
| Sun 15 Nov | Phase 3: system check, Start Session dialog, fixture profiles | Converted fine-tunes run through `wb-engine batch --file` | |
| Mon 16 Nov | Phase 3 merged; build the gate pack (`docs/prompts/laptop-gate-pack.md`) | | |
| Tue 17 Nov | `/run-phase 4` if the laptops are late | | Laptops arrive: run the gate pack on both |

If a day slips, keep the order and push later phases back; never skip a gate. Long jobs (CUDA builds, soaks, downloads) go overnight.

## 5. Legion setup (Sunday)

1. From a normal (not administrator) PowerShell window, run `powershell -ExecutionPolicy Bypass -File .\scripts\setup-legion.ps1 -Update -InstallMissing` from the repository folder. It checks Git, Rust, Node.js LTS, pnpm, Visual Studio Build Tools (C++ workload), CMake, LLVM and Claude Code, updates what is outdated, installs what is missing, and writes `.setup/report.txt` (see `docs/SETUP.md`).
2. Before Phase 2 (Thursday), run it again with `-IncludePhase2` to add the Vulkan SDK and the CUDA Toolkit.
3. Install VB-CABLE (virtual audio cable) for soak and replay tests.
4. Set the GPU mode to Hybrid and check that both GPUs appear.
5. Create an empty Git repository, copy the kit into it (including the hidden `.claude/` folder) and commit.
6. Open Claude Code in the repository and type `/run-phase 0`.

## 6. Running two sessions

1. Main session (Track A) works in the repository folder on `main` and the phase branches.
2. For Track B, create a worktree once Phase 0 is merged: `git worktree add ../stelmar-eval phase-5a-eval-groundwork`, open a second Claude Code session there and type `/run-phase 5a`.
3. Track B touches only `eval/`, `scripts/models/` and `models/README.md`, so the two sessions don't edit the same files. Merge Track B into `main` when its gate passes.

## 7. When the laptops arrive

1. Copy `gate-pack/` to each laptop (USB drive) and run `run-gates.ps1` as administrator. It needs no development tools.
2. Copy each laptop's `gate-results-<machine>.zip` back to the Legion and run `scripts/ingest-gate-results.ps1`, which updates `docs/GATES.md`.
3. Fix any FAIL in its own phase before merging further work. Record the laptops' names and specs in `docs/GATES.md`.
