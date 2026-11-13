# Development setup (the Legion)

All development and release builds happen on the Legion (Lenovo Legion Pro 5 16IRX9: Core i9-14900HX, 32 GB RAM, RTX 4070 Laptop GPU 8 GB, Intel UHD integrated GPU, Windows 11).

## The setup script

Run it from a normal PowerShell window, not as administrator; it stops if elevated. Installers that need administrator rights ask for approval themselves, and an elevated run would leave `.git` owned by Administrators, which Git then refuses to use from your normal account.

```powershell
# From the repository folder
powershell -ExecutionPolicy Bypass -File .\scripts\setup-legion.ps1 -Update -InstallMissing
# Before Phase 2, add the GPU SDKs
powershell -ExecutionPolicy Bypass -File .\scripts\setup-legion.ps1 -Update -InstallMissing -IncludePhase2 -NoGit
```

| Switch | Effect |
| --- | --- |
| (none) | Checks and reports only, then sets up Git |
| `-Update` | Upgrades installed tools that have a newer release (winget, `rustup update`, `pnpm self-update`, `claude update`) |
| `-InstallMissing` | Installs missing Phase 0 tools with winget (Windows may ask for administrator approval) |
| `-IncludePhase2` | Also checks or installs the Vulkan SDK and the CUDA Toolkit |
| `-NoGit` | Skips the Git steps |
| `-NoTools` | Skips the tool checks and runs only the Git steps |

The Git steps create the repository on branch `master` if it doesn't exist, enable `.githooks`, set `origin` to `https://github.com/thenamakop/stelmar_voice_workbench.git`, and make the first commit with your own Git identity. They never push. The report is written to `.setup\report.txt` (git-ignored).

## What each phase needs

| Tool | Needed from | Why |
| --- | --- | --- |
| Git | Phase 0 | Version control |
| Rust (rustup; the version pinned in `rust-toolchain.toml`) | Phase 0 | Tauri app and engines |
| Node.js LTS and pnpm | Phase 0 | React front end and Tauri CLI |
| Visual Studio Build Tools, C++ workload | Phase 0 | MSVC compiler and Windows SDK for Rust and whisper.cpp |
| WebView2 runtime | Phase 0 | Tauri's web view (built into Windows 11) |
| CMake | Phase 2 (installed early) | Builds whisper.cpp |
| LLVM (libclang) | Phase 2 (installed early) | `bindgen` for whisper-rs |
| Python 3 | Phase 5a | Evaluation and model conversion scripts |
| VB-CABLE | Phase 1 | Virtual audio cable for soak and replay tests (install by hand from vb-audio.com; reboot) |
| Vulkan SDK | Phase 2 | GPU engine build for integrated and discrete GPUs |
| CUDA Toolkit | Phase 2 | NVIDIA engine build; pick a version the bundled ggml supports |
| Claude Code | Phase 0 | Runs the phases |
| GitHub CLI (optional) | Any | Pull requests from the terminal |

## Machine settings

1. **GPU mode: Hybrid.** In Lenovo Vantage (or the BIOS), set the GPU working mode to Hybrid and reboot, so the Intel integrated GPU is visible. The Legion emulation profiles use it (`docs/LEGION-FIRST.md`).
2. **Power:** on mains, Lenovo Vantage performance mode, for builds and benchmarks.
3. **Disk:** the repository lives on `D:\Projects\stelmar_voice_workbench` (NTFS or a Dev Drive). Keep about 100 GB free there: Rust build folders for four engine variants (15 to 25 GB), converted models (15 to 20 GB), original checkpoints during conversion (10 to 15 GB), evaluation datasets (5 to 10 GB), the Python environment (4 to 6 GB) and test recordings. Models, datasets, audio and build output are git-ignored.
4. **Caches on D:** set `HF_HOME=D:\caches\huggingface` so model downloads don't fill C:. pnpm creates its own store on D: automatically (hard links need the store on the same drive as the project).
5. **Antivirus:** on a normal NTFS drive, add a Microsoft Defender exclusion for the repository's `target` folder, or use a Dev Drive, whose performance mode speeds up Rust builds.

## Fixing discrepancies

`scripts/fix-legion.ps1` fixes what the 8 November 2026 report found (Node.js 26 LTS, pnpm, LLVM on PATH, Visual Studio 2026 Build Tools, Python, CMake, Vulkan SDK, CUDA Toolkit). It asks before each step. Run it from a normal PowerShell window, then open a new window and run `setup-legion.ps1 -NoGit` to refresh the report.

## Drivers (update by hand)

| Driver | Where |
| --- | --- |
| NVIDIA GeForce (RTX 4070 Laptop) | NVIDIA app, or https://www.nvidia.com/en-us/drivers/ |
| Intel UHD Graphics (14th Gen) | Intel Driver & Support Assistant: https://www.intel.com/content/www/us/en/support/detect.html (or Lenovo Vantage for the Lenovo-validated version) |

Install drivers while online, before Phase 2: the Vulkan engine build depends on them.

## After a fresh clone

```powershell
git config core.hooksPath .githooks
```

This enables the hook that strips AI attribution from commit messages (`CONTRIBUTING.md`).

## Stopping what the coding agent starts

Everything the agent runs on the Legion is an ordinary process of your Windows user, in your own desktop session (session 1,
`Console`, as `tasklist` shows), not in a sandbox or another account. You can see and stop all of it from a normal window: Task
Manager, or a normal PowerShell. Checked on 10 Nov 2026, when you stopped a stuck conversion (python, pid 38104) and its children.
The agent's own permission layer can refuse to kill a process for it; that never prevents you from doing it.

Long jobs run in the background (cargo, CMake and CUDA builds, model downloads, conversions and quantisation, gate runs, evaluation
runs). To see them and stop them:

```powershell
# What is running: engines, builds, Python jobs, the gate rig, the app
Get-Process cargo, rustc, cmake, cl, link, nvcc, python, wb-engine*, wb-emulate, wb-testbench, stelmar-voice-workbench -ErrorAction SilentlyContinue |
  Select-Object Name, Id, StartTime, @{n='CPU_s';e={[int]$_.CPU}}
# Stop one, with its children
taskkill /F /T /PID <pid>
# Stop an emulated gate run: ending wb-emulate ends everything inside its Job Object
taskkill /F /T /IM wb-emulate.exe
```

Each job that must not overlap another leaves a lock file; if a job was killed, remove its lock by hand (the scripts remove their
own on a normal end or an error):

| File | Held by |
| --- | --- |
| `D:\Projects\stelmar-gate.lock` and `stelmar-gate-request.flag` | a timing-sensitive gate run, or the request for a gate sequence |
| `D:\Projects\stelmar-build.lock` | `scripts\build-engines.ps1` (a running build) |
| `D:\Projects\stelmar-eval-busy.lock` | a heavy evaluation, conversion or quantisation step |

The engines the app starts are in the app's kill-on-close Job Object: ending the app ends them. Never delete a lock while its process
is still alive: a waiting step may take it and start work (10 Nov 2026).
