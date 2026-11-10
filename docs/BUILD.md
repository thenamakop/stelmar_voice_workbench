# Building the app and the speech engines

Everything here runs on the Legion (Windows 11). Nothing in a build or at runtime reaches the network after the first
`git clone` and the first `pnpm install` / `cargo fetch`: whisper.cpp is vendored in `third_party/` as a copied tree, models
are never downloaded by the app, and the engines open no sockets (checked by `offline-auditor`).

## Tools

| Tool | Used for | Notes |
| --- | --- | --- |
| Git, Rust stable (1.88 or newer), Node.js LTS, pnpm | app, UI, scripts | `rust-version` is 1.88 because whisper-rs-sys 0.15 needs it |
| Visual Studio Build Tools (C++ workload, Windows 11 SDK) | MSVC for Rust and for ggml | `cl.exe`, `link.exe`, `dumpbin.exe` (used by `scripts/check-no-zmm.ps1`) |
| CMake 3.20 or newer | whisper.cpp / ggml | found on `PATH` |
| LLVM (libclang) | `bindgen` in whisper-rs-sys | `LIBCLANG_PATH` points at `C:\Program Files\LLVM\bin` |
| Vulkan SDK (`VULKAN_SDK`) | `vulkan` engine | the loader library `vulkan-1.lib`, headers and `glslc` |
| CUDA Toolkit (`CUDA_PATH`) | `cuda` engine | `nvcc`, cuBLAS; the Legion has 13.5 |

`scripts/setup-legion.ps1 -Update -InstallMissing -IncludePhase2` installs what is missing (`docs/SETUP.md`).

## Engine variants

`wb-engine` is one crate built four ways. Every build sets `GGML_NATIVE=OFF`, so no flag comes from the build machine's own CPU
(AGENTS.md non-negotiable 4). `scripts/build-engines.ps1` sets the flags below through environment variables, which
whisper-rs-sys forwards to CMake, and then reads each build's `CMakeCache.txt` and configure log back: the build fails if a flag or
a backend line is missing.

| Variant | whisper-rs feature | CPU flags | Backend |
| --- | --- | --- | --- |
| `avx2` | none | `GGML_AVX=ON GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON`, `GGML_AVX512=OFF` | CPU |
| `avx512` | none | the `avx2` flags plus `GGML_AVX512=ON` | CPU |
| `vulkan` | `vulkan` | the `avx2` flags | Vulkan (any GPU with a Vulkan driver, integrated ones included) plus CPU |
| `cuda` | `cuda` | the `avx2` flags | CUDA plus CPU |

Outputs are `src-tauri\binaries\wb-engine-<variant>-x86_64-pc-windows-msvc.exe` (the name Tauri's `externalBin` expects), each built in
its own target directory `target\engines\<variant>`. They are git-ignored.

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-engines.ps1                 # all four
powershell -ExecutionPolicy Bypass -File scripts\build-engines.ps1 -Variants avx2  # one
```

A build takes the lock `D:\Projects\stelmar-build.lock` and refuses to start while a gate holds `D:\Projects\stelmar-gate.lock`.
In the other direction the gate scripts refuse to start while the build lock exists or a compiler is running, so a build never spoils
a timing-sensitive run (`scripts/lib/GateRun.psm1`).

### The AVX-512 scan

CI builds `avx2` and fails if its disassembly contains a `zmm` register (`scripts/check-no-zmm.ps1`, which runs
`dumpbin /disasm`). A portable build must not contain an instruction a laptop without AVX-512 would fault on.

### Runtime checks

Before an engine touches whisper.cpp it checks that the CPU has what its build was compiled for (AVX2, FMA and F16C; AVX-512 F, CD, VL, DQ
and BW for `avx512`) and exits with code 3 and a clear message if not. The system check (Phase 3) runs `wb-engine-<variant> info` for each
variant in its own process, so a build that cannot run on a machine is marked unusable and the app is unaffected.

## whisper.cpp

`third_party/whisper.cpp` is a copy of the whisper.cpp **v1.9.5** release, pinned to one commit and one tree hash; see
`third_party/README.md` for the commit, the file list, the size and why it is vendored. whisper-rs-sys 0.15.0 (the crate whisper-rs 0.16
uses) bundles whisper.cpp 1.8.3; the Phase 5a evaluation harness runs whisper.cpp 1.9.5's `whisper-cli`, and the "within 3 WER points"
comparison between the two is only meaningful with the same version on both sides. A vendored copy of whisper-rs-sys
(`third_party/whisper-rs-sys`, with `build.rs` pointed at `third_party/whisper.cpp` and nothing else changed) is wired in through
`[patch.crates-io]` in the root `Cargo.toml`.

Checks, all run by CI:

* `pnpm check:vendor` recomputes the tree hash and fails if it differs from the pin.
* `scripts/check-whisper-flags.ps1` checks that every CMake option the build sets still exists under that name in the 1.9.5 CMake files, so a
  renamed flag cannot silently drop a backend.
* The engine's `build.rs` computes the same hash at build time and fails the build on a mismatch. `wb-engine info`, `bench` and `batch`
  print the whisper.cpp version, commit and tree hash they were built with.

To update the vendored copy: change the tag and commit in `scripts/vendor-whisper-cpp.ps1`, run it, put the new hash into
`third_party/README.md`, and repeat the 3-WER comparison.

## Licence exception

AGENTS.md allows MIT, Apache 2.0, BSD, ISC and OFL dependencies. `whisper-rs` and `whisper-rs-sys` are under the Unlicense (a public-domain
dedication, at least as permissive as MIT). The project owner approved them as a **named exception for these two crates only** on
10 November 2026; the allowed list in AGENTS.md is unchanged. If a licence checker such as `cargo-deny` is added, the exception goes in as a
per-crate entry, never as a global allow. The licence texts ship in `THIRD_PARTY_LICENCES.md`.

## Offline use

After the first fetch, `cargo build --offline` and `pnpm install --offline` work. Model files and evaluation data are never part of a build;
they live under `models/` and `eval/data/`, which are git-ignored (`models/manifest.json` holds only names, sizes and SHA-256 values).
