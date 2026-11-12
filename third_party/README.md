# Vendored sources

Two things are copied here so that builds work offline after the first clone and every engine is built from the same reviewed bytes.
Nothing in this folder is fetched at build time.

## whisper.cpp

The speech engines (`wb-engine`) run whisper.cpp through whisper-rs. whisper-rs-sys 0.15.0, the crate whisper-rs 0.16 depends on, bundles
whisper.cpp 1.8.3. The Phase 5a evaluation harness runs the whisper.cpp 1.9.5 `whisper-cli`, and the Phase 2 comparison "within 3 WER
points" is only meaningful with the same version on both sides, so the engines are built from 1.9.5.

tag: v1.9.5
commit: d1be6fde11ac6e0407606b4e42fe72d34add8037
tag-object: d2a764c648779feb36d44363f212a29b00aad1af
source: https://github.com/ggml-org/whisper.cpp (cloned once, at development time, by `scripts/vendor-whisper-cpp.ps1`)
tree-sha256: e736b4c9a5be2bc0e9b294102e261fe69c2a439976652ff6107b0c361f8667fb
files: 646
size: 10.6 MB

* **A copied tree, not a submodule.** It is committed as ordinary files (`.gitattributes` marks `third_party/**` as `-text`, so Git never rewrites
  line endings and the hash is the same on every checkout).
* **What is in it** (the same list whisper-rs-sys ships, less the backends we never build): `CMakeLists.txt`, `LICENSE` (MIT, kept), `cmake/`,
  `include/` (`whisper.h`, `parakeet.h`), `src/`, `ggml/CMakeLists.txt`, `ggml/cmake/`, `ggml/include/*.h`, `ggml/src/` with the CPU, CUDA and Vulkan backends and the core,
  `bindings/CMakeLists.txt` and `bindings/javascript/package-tmpl.json`.
* **What is not:** models, samples, examples, tests, scripts, binaries, and the ggml backend folders for BLAS, CANN, ET, Hexagon, HIP, Metal,
  MUSA, OpenCL, OpenVINO, RPC, SYCL, VirtGPU, WebGPU, zDNN and ZenDNN. Each is switched on by a CMake option the project leaves off. Without
  the pruning the tree was 1,456 files and 22.7 MB; the 1.8.3 copy inside whisper-rs-sys is 778 files and 13 MB.
* **The hash** is SHA-256 over, for every file in ordinal byte order of its relative path (separator `/`): the path bytes, one 0x00 byte, the
  file length as 8 bytes little-endian, then the file bytes. `pnpm check:vendor` (`scripts/check-vendor.mjs`) recomputes it, and
  `crates/wb-engine/build.rs` computes it again at build time and stops the build on a mismatch. `wb-engine info`, `bench` and `batch` print
  the version, commit and hash they were built with.
* **Updating:** change the tag and commit in `scripts/vendor-whisper-cpp.ps1`, run it, put the new `tree-sha256` and `files` here, run
  `scripts/check-whisper-flags.ps1`, rebuild the engines and repeat the 3-WER comparison.
* **Licence:** MIT (`third_party/whisper.cpp/LICENSE`), listed in `THIRD_PARTY_LICENCES.md` which ships with the app.

## whisper-rs-sys

A copy of whisper-rs-sys 0.15.0 (the registry crate, unpacked), wired in with `[patch.crates-io]` in the root `Cargo.toml`. Licence: the
Unlicense (`third_party/whisper-rs-sys/UNLICENSE`); the project owner approved it, and whisper-rs, as a named exception on 10 November 2026
(`docs/BUILD.md`).

Changes from the published crate, all in `build.rs`, none in `src/` (three, each a few lines):

1. The whisper.cpp folder is `../whisper.cpp` (the tree above) instead of a `whisper.cpp` folder inside the crate: the copy source
   (`fs_extra::dir::copy`), the bindgen `-I` paths and the two optional backend headers.
2. The copy into the build output folder is refreshed whenever the vendored tree changes, and cargo is told to watch it (the published crate copies once, so a changed tree silently builds from the old copy).
3. `get_whisper_cpp_version` also reads the three `set(WHISPER_VERSION_MAJOR|MINOR|PATCH n)` lines: whisper.cpp 1.9.5 no longer has the
   `project("whisper.cpp" VERSION x.y.z)` line the published build script looks for, and without this the build panics.

`Cargo.toml` is the published one, unchanged (its `include` list only matters when packaging).

The safe wrapper, `whisper-rs` 0.16.0, is used from the registry unchanged.
