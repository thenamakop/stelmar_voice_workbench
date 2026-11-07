@AGENTS.md

# Claude Code notes

- Never add yourself as a contributor: no `Co-Authored-By` trailer, no "Generated with Claude Code" line, no session link in commits or pull request descriptions (AGENTS.md non-negotiable 8). `.claude/settings.json` sets `attribution` to empty strings; keep it that way, and don't pass `--no-verify` (the `commit-msg` hook is the backstop).
- Start each phase with `/run-phase <n>` (for example `/run-phase 0`). Propose a plan and wait for approval before editing files.
- Reviewer subagents live in `.claude/agents/`: `design-consistency-reviewer`, `asr-quality-auditor` and `offline-auditor`. Use them when AGENTS.md says to; a phase isn't done until they report PASS or their findings are fixed.
- You run on the Legion (Windows 11, RTX 4070 Laptop GPU 8 GB). The room machines are undecided 16 GB laptops that will differ by room. When a gate needs the low-baseline or mid laptop, write a PowerShell script that runs there without development tools (copy the built app or engines, run, collect a log), tell the person to run it on each laptop, and wait for the logs.
- The test laptops are not available before 17 November 2026. Until then follow `docs/LEGION-FIRST.md`: run laptop gates under `wb-emulate --profile emu-low` or `emu-mid`, record them in `docs/GATES.md`, and keep `wb-emulate` out of every bundle. `/run-phase gate-pack` builds the pack the laptops run on arrival.
- Release engine builds set `GGML_NATIVE=OFF` and the per-build `GGML_*` (and, for the experimental OpenVINO build, `WHISPER_*`) flags through `scripts/build-engines.ps1`; whisper-rs-sys forwards these environment variables to CMake. Never measure speed with a native build and ship a portable one.
- CUDA and Vulkan builds of whisper.cpp take many minutes, and benchmarks, soak tests and day simulations take hours. Run them in the background and keep working on something else.
- Prefer PowerShell commands, and say so when a step needs a specific machine, an audio device, a GPU driver or an installer.
- Model and dataset downloads are gigabytes. Announce them before starting and put files under `models/` or `eval/data/`, which are git-ignored.
