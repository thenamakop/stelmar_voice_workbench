# STELMAR Voice Workbench: rules for coding agents

You are building an offline desktop app that records every STELMAR 2027 session and turns it into transcripts on the same computer, with no network. English and Hindi are the primary languages and speakers switch between them at any point, even mid-sentence; Urdu is also supported. The venue computers are undecided 16 GB laptops that differ by room, so the main path is record + post-process: each closed 10-minute part is transcribed in the background on the CPU, the GPU (integrated or discrete) or both, while recording continues. Live text is off by default everywhere and is chosen per recording in the Start Session dialog, only where the first-launch system check has cleared the machine. The app looks like the STELMAR website. The plan is `docs/PLAN.md`; phase prompts are in `docs/prompts/`; run them in order, one per branch.

## Non-negotiables

1. **Offline at runtime.** The app and its engine processes never open a network connection: no HTTP clients, telemetry, crash reporters, updaters, CDNs, remote fonts or model downloads. Models, fonts, icons and test clips are local files. Scripts under `scripts/` and `eval/` may download at development time; the app may not.
2. **The recording comes first.** Audio reaches disk before any processing and survives a crash (WAV header flushed every second). Transcription runs in separate engine processes at lower priority. Nothing they do may block, slow or drop the recorder: when memory or CPU runs short, the queue pauses, never the recorder.
3. **Measured, not assumed.** Tiers, modes, thread counts and model choices come from the system check on the machine itself and from `eval/` results. Never hard-code a model or a speed for a class of machine, and never lower post-processing quality to fix speed without approval.
4. **Portable builds.** Release engine builds never use build-machine-native CPU flags (`GGML_NATIVE=OFF`). Every engine checks the CPU features it needs at runtime before it touches whisper.cpp.
5. **Same design as the website.** Use only `design/tokens/*.css`, `design/fonts.css` and components ported from `design/reference/ds-bundle.js`. No Tailwind, CSS-in-JS libraries, component kits, icon sets other than lucide-static 0.460.0, or new colours. Match `design/mockups/`.
6. **Languages are first-class, and they mix.** Never assume one language per session: detect the language per sentence-sized chunk among the session's expected languages (default English + Hindi), never force English onto Hindi speech, and keep each word in its script (Hindi in Devanagari, English in Latin, Urdu in right-to-left Nastaliq). Script-aware normalisation in evaluation. No Latin-only shortcuts. *(13 Nov 2026: Hindi and Urdu are one spoken language with an organiser-chosen output script, Devanagari by default; see `docs/GATES.md`, Open items, row dated 13 Nov 2026.)*
7. **Live is opt-in.** Every recording starts in Record + post-process unless the operator picks live text in the Start Session dialog, and the dialog offers live only where `capability.json` clears it for all of the session's languages.
8. **No AI attribution in Git or on GitHub.** Commits, pull requests, issues and release notes never credit an AI tool: no `Co-Authored-By` trailers for Claude, Anthropic or any other assistant, no "Generated with" lines, no session links, and no AI tool as author or committer. Commits use the developer's own Git identity. `.claude/settings.json` turns Claude Code's attribution off, `.githooks/commit-msg` strips any that slips through, and CI rejects pushes that contain it. This applies to Claude Code, Claude and every other agent working on this repository.

## Stack (pinned)

| Area | Choice |
| --- | --- |
| Shell | Tauri 2.11.x (Rust stable) with React, TypeScript (strict) and Vite; pnpm |
| Engines | `wb-engine` (Rust, whisper-rs 0.16 with bundled whisper.cpp, Silero VAD v6.2.0 through `WhisperVadContext`), built as `avx2`, `avx512`, `vulkan` (integrated and discrete GPUs) and `cuda` variants, plus an experimental `openvino` variant; an engine can hold two models for per-language routing; shipped as Tauri `externalBin` sidecars and started from Rust with `std::process` (no shell plugin) |
| Engine protocol | `crates/wb-protocol`: versioned serde types; subcommands `info`, `bench`, `batch`, `live`; JSON lines on stdout, logs on stderr; 16 kHz mono f32 PCM on stdin for `live` |
| Audio | cpal (capture), ringbuf (lock-free), hound (WAV), rubato (resampling to 16 kHz) |
| System probe | sysinfo; `windows` crate (DXGI adapters, power status, MMCSS, Job Objects, process priority); nvml-wrapper for NVIDIA when `nvml.dll` loads |
| Data | rusqlite with the `bundled` feature (WAL), append-only JSONL, serde |
| Logs | tracing, written to the session folder and an app log; no remote sinks |
| Tests | cargo test, Vitest, Playwright against the web UI with a mocked Tauri bridge; Python only in `eval/` and `scripts/` |

## Repository layout

```text
AGENTS.md  CLAUDE.md  README.md  CONTRIBUTING.md
.claude/agents/            reviewer subagents (design, ASR quality, offline)
.claude/skills/run-phase/  the /run-phase command
.claude/settings.json      project settings (AI attribution off)
.githooks/commit-msg       strips AI attribution from commit messages
.github/workflows/         CI, including the no-AI-attribution check
docs/PLAN.md               the plan: decisions D1 to D14, tiers, system check, languages, queue, gates
docs/LEGION-FIRST.md       work plan until the test laptops arrive; emulation profiles; gate statuses
docs/GATES.md              gate ledger: every machine-named Verification item and its status
docs/SETUP.md              Legion build tools; docs/TEST-AUDIO.md: local test sets (after Phase 4)
docs/prompts/              phase-0 ... phase-8 prompts, phase-5a (parallel track), laptop-gate-pack
docs/programme.json        STELMAR 2027 programme (seeds the Sessions screen)
design/                    frozen tokens, offline fonts, brand assets, DS reference, mockups
models/                    manifest template and README (model files are not committed)
src/                       React UI (src/ds = ported design-system components)
src-tauri/                 Rust app: audio, recorder, store, scheduler, system check, engine supervisor, commands
crates/wb-protocol/        engine protocol types shared by the app and the engines
crates/wb-engine/          engine binary: info, bench, batch, live
crates/wb-emulate/         development-only launcher that emulates the test laptops on the Legion (never bundled)
resources/bench/           bundled 60-second test clips per language (FLEURS test, CC BY 4.0)
eval/                      evaluation harness and REPORT.md
scripts/                   setup-legion.ps1; build-engines.ps1; dev-time model download, conversion and quantisation
```

## Engineering rules

- **Audio callback:** copy samples into the ring buffer and return. No allocation, locks, logging or I/O.
- **Process model:** the app never links whisper.cpp or whisper-rs; all inference happens in engine processes inside a Windows Job Object. The recorder thread runs with raised priority (MMCSS "Pro Audio"); the live engine at normal priority; the batch engine at below-normal priority with the thread count from `capability.json`.
- **Memory:** never start an engine if free RAM after its measured peak would fall under 3 GB; pause the batch engine when free RAM drops under 2 GB.
- **Single writer:** only the app writes transcript files. Every committed segment is appended to its revision's JSONL and fsynced before it is shown anywhere.
- **Resumable jobs:** progress is a `committed_until_ms` cursor; restarting an engine never repeats or loses a committed segment. Cuts happen only at VAD pauses.
- **Work units and devices:** jobs are cut into units of about 5 minutes at VAD pauses; in the hybrid configuration a CPU engine and a GPU engine work on different units at once, and the app commits segments in audio order. Integrated GPUs share RAM with the CPU: count their memory in every memory rule.
- **Languages:** every segment records `lang`, `lang_prob` and `mixed`; operator locks (Alt+1/2/3) and returns to detection (Alt+4) are logged with audio times and followed by post-processing.
- **Time:** segment times are milliseconds from session start on the audio clock. Wall-clock time appears only in the UI and in IST.
- **Errors:** no `unwrap()` or `expect()` on production paths; use `thiserror` and `anyhow`; surface errors with the DS `Toast`, never silently. An engine failure is reported and retried; it never takes the app down.
- **Windows first:** PowerShell-friendly scripts; build paths with `std::path` and `path.join`; no bash-only commands in `package.json`.
- **Evidence files** under `docs/evidence/` are written by scripts only, never edited by hand. To correct one, fix the script and re-run it. A person's or an agent's reading of a screenshot or log goes in a separate note file that says who wrote it.
- **Do not edit** `design/tokens/{colors,typography,spacing,effects,base}.css`, anything in `design/reference/`, or the decisions in `docs/PLAN.md` (ask instead). Never commit model binaries or private audio.
- **Licences:** only MIT, Apache 2.0, BSD, ISC or OFL dependencies and models without asking (system SDKs such as the CUDA Toolkit, the Vulkan SDK and WebView2 excepted). Record each model's source, licence and SHA-256 in `models/manifest.json`.

## Workflow

- One phase per branch (`phase-<n>-<slug>`) and per pull request. Plan first, then build.
- Before calling a phase done, run `pnpm lint && pnpm typecheck && pnpm test` and `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, then every item in the phase's Verification list. Paste the results.
- Three reference machines: the **Legion** (build machine, RTX 4070 Laptop GPU), the **low-baseline laptop** (the weakest 16 GB laptop available: 4 cores, basic integrated graphics) and the **mid laptop** (16 GB, newer integrated GPU). The room machines are not known yet, so nothing may assume one model. Speed, memory, load and endurance gates must pass on the low-baseline laptop; hybrid CPU + GPU gates on both laptops; GPU live gates on the Legion. Every number you report names the machine, engine build, threads and method.
- Until the laptops arrive (not before 17 November 2026): laptop gates run on the Legion under the `emu-low` and `emu-mid` profiles of `crates/wb-emulate` and are recorded as PROVISIONAL in `docs/GATES.md`, with 25% speed headroom; only the items `docs/LEGION-FIRST.md` section 3 lists may be DEFERRED. Phase 5a runs in a parallel git worktree and touches only `eval/`, `scripts/models/` and `models/README.md`. When the laptops arrive, the gate pack re-runs every PROVISIONAL and DEFERRED item on them.
- UI changes: attach a 1536x864 screenshot beside the matching mockup and have the `design-consistency-reviewer` subagent check it.
- Engine, model, scheduler, system-check, language or evaluation changes: have the `asr-quality-auditor` subagent check them.
- Before any merge: have the `offline-auditor` subagent check the change.
- Blocked by hardware, a licence, a missing file or a decision: stop and write exactly what you need. Don't invent a workaround that breaks a non-negotiable.
