# STELMAR Voice Workbench

An offline Windows desktop app that records every session of the STELMAR 2027 Scholars' Conference (BML Munjal University, 8 and 9 February 2027) and turns the recordings into transcripts on the same computer, with no internet connection.

- **Records first.** Audio goes to disk in crash-safe 10-minute WAV parts and is mirrored to a second drive before anything else touches it.
- **Transcribes on ordinary laptops.** The room machines are undecided 16 GB laptops, so the default mode is Record + post-process: each closed part is transcribed in the background on the CPU, the integrated GPU or both, while recording continues.
- **Speaks the room's languages.** English and Hindi are the primary languages, and speakers may switch between them at any point, even mid-sentence. Urdu is also supported. The language is detected sentence by sentence.
- **Measures before it decides.** A first-launch system check benchmarks the machine and picks the models, devices and modes it can sustain.
- **Live text is opt-in.** It is off by default and chosen before each recording in the Start Session dialog, only on machines the system check has cleared.
- **Looks like the STELMAR website.** It uses the website's design tokens, fonts and components.

Speech recognition uses [whisper.cpp](https://github.com/ggml-org/whisper.cpp) through whisper-rs, with Silero voice activity detection and Whisper models fine-tuned for Indian languages.

## Status

Planning is complete and development is starting. Work follows nine phases (0 to 8), each with a prompt in `docs/prompts/` and a gate that must pass before the next phase starts. Until the test laptops arrive (not before 17 November 2026), the Legion build machine emulates them; see `docs/LEGION-FIRST.md`.

| Phase | Delivers |
| --- | --- |
| 0 | App shell in the STELMAR design |
| 1 | Crash-safe recording, parts index, backup mirror, Legion emulation launcher |
| 2 | Engine processes for CPU and GPU, per-chunk language detection, model registry, supervisor |
| 3 | System check, capability profile, Start Session dialog |
| 4 | Post-processing queue: work units, hybrid CPU + GPU, rolling and night passes |
| 5a | Evaluation groundwork (runs in parallel with Phases 1 to 3) |
| 5 | Code-switching, Hindi and Urdu quality, evaluation, model choice |
| 6 | Sessions, Review editor, exports, archives |
| 7 | Opt-in live and near-live text, projector caption window |
| 8 | Offline proof, full-day simulations, installers, model packs |

Progress against each gate is tracked in `docs/GATES.md`.

## How it works

```text
Mixer line out -> USB audio interface -> App (Tauri 2, Rust + React)
                                          |- Recorder: 48 kHz WAV parts, flushed every second, mirrored
                                          |- Scheduler: work units of ~5 minutes, cut at pauses
                                          |     |- wb-engine (CPU build)   -+
                                          |     |- wb-engine (GPU build)   -+-> segments, committed in audio order
                                          |- Store: SQLite + append-only JSONL per revision
                                          |- UI: Sessions, Record, Queue, Review, System, Settings, Captions
```

The app never links whisper.cpp itself. Every model runs in a separate `wb-engine` process at lower priority, so a model crash, a GPU driver fault or running out of memory can stop an engine but never the recording.

## Repository layout

| Path | Contents |
| --- | --- |
| `AGENTS.md` | Rules for coding agents: non-negotiables, pinned stack, engineering rules, workflow |
| `CLAUDE.md` | Claude Code notes; imports `AGENTS.md` |
| `CONTRIBUTING.md` | Branches, commits, reviews and the no-AI-attribution rule |
| `.claude/` | `/run-phase` command, reviewer subagents, project settings |
| `.githooks/` | `commit-msg` hook that strips AI attribution |
| `.github/workflows/` | CI check that rejects AI attribution |
| `docs/` | Plan, Legion-first schedule, gate ledger, setup guide, test-audio guide, phase prompts, programme |
| `design/` | Frozen STELMAR tokens, offline fonts, brand assets, design-system reference, mockups |
| `models/` | Model manifest template and conversion notes (model files are never committed) |
| `scripts/` | `setup-legion.ps1`; later build, model and gate scripts |

Source folders (`src/`, `src-tauri/`, `crates/`, `eval/`) are created by the phases.

## Getting started (development)

Development happens on Windows 11 on the Legion (Core i9-14900HX, 32 GB, RTX 4070 Laptop GPU).

1. Clone the repository and open a normal PowerShell window (not administrator) in it.
2. Run the setup script. It checks and updates the build tools, then sets up Git (it only commits when nothing is committed yet):

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\scripts\setup-legion.ps1 -Update -InstallMissing
   ```

3. Read the report in `.setup\report.txt` and fix anything marked missing (`docs/SETUP.md` explains each item).
4. Start Claude Code in the repository and type `/run-phase 0`.

After a fresh clone, run `git config core.hooksPath .githooks` once if you skip the script.

## Viewing the mockups

Browsers block local fonts over `file://`. From the repository folder run a static server (`npx serve .` or `python -m http.server`) and open `/design/mockups/start-session.html`, `record-only.html`, `record-live.html`, `system-check.html` or `captions.html`. PNG renders sit beside them; their numbers are placeholders.

## Documentation

See `docs/README.md` for the full index. The most useful starting points are `docs/PLAN.md` (the plan), `docs/LEGION-FIRST.md` (the work until 17 November) and `AGENTS.md` (the rules every contributor and agent follows).

## Contributing

Read `CONTRIBUTING.md`. In short: one phase per branch, gates before merges, and every commit under a human author's own Git identity, with no AI tool credited anywhere in the history.

## Licence

Not yet chosen; to be decided by the STELMAR 2027 organising team. Third-party components keep their own licences (MIT, Apache 2.0, BSD, ISC and SIL OFL only), listed in `THIRD_PARTY_LICENCES.md` from Phase 8.
