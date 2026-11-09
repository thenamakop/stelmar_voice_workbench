# Unsafe code

The workspace denies `unsafe_code` (`Cargo.toml`, `[workspace.lints.rust]`). Only the modules
listed here may allow it, each with `#![allow(unsafe_code)]` at the top of a file named `win.rs`.
Clippy denies `undocumented_unsafe_blocks` and `multiple_unsafe_ops_per_block`, so every unsafe
block holds exactly one operation and has a `// SAFETY:` comment saying why it is sound. Logic that
does not need `unsafe` (parsing buffers, deciding what to do) lives in safe modules beside them.

One related exception to the audio-callback rule ("copy samples and return") lives in safe code:
`crates/wb-recorder/src/capture.rs` `register_capture_thread` registers the driver's capture
thread with MMCSS "Pro Audio" on the first callback only (through `sys.rs` and `win.rs`); every
later callback returns at its first line.

Adding a module to this list needs the same review as a dependency: say what Windows API it calls
and why no safe crate does it.

| Module | Shipped in the app | Unsafe blocks | Calls |
| --- | --- | --- | --- |
| `crates/wb-recorder/src/win.rs` | Yes (recorder library) | 10 | `AvSetMmThreadCharacteristicsW` / `AvRevertMmThreadCharacteristics` (MMCSS "Pro Audio" for the recorder thread), `SetThreadPriority` (fallback and below-normal for helper threads), `SetThreadExecutionState` (keep the machine awake during a session), `GetThreadTimes` (recorder CPU time), `GetDiskFreeSpaceExW` (free space on the recording drive), `GetDriveTypeW` (refuse network drives for recordings and backups) |
| `src-tauri/src/win.rs` | Yes (app) | 4 | WebView2 `CoreWebView2`, `add_ProcessFailed` and `ProcessFailedKind` (display guard), `Reload` (after a renderer failure) |
| `crates/wb-emulate/src/win.rs` | No (development only) | 32 | `GetLogicalProcessorInformationEx` (cores by efficiency class), `CreateJobObjectW` / `SetInformationJobObject` / `AssignProcessToJobObject` / `CloseHandle` (the emulation job), `GetProcessAffinityMask`, `VirtualAlloc` / `VirtualFree` (commit probe), Vulkan loader through `ash` (device list with LUIDs and drivers for `GGML_VK_VISIBLE_DEVICES`), `LoadLibraryW` / `GetProcAddress` on nvcuda.dll and the CUDA driver API (`cuInit`, `cuDeviceGetCount`, `cuCtxCreate` for the probe that shows CUDA is hidden inside the job) |
| `crates/wb-testbench/src/win.rs` | No (development and gate pack) | 42 | DXGI adapter list and a Direct3D 11 compute load (GPU stress), `D3DCompile`, Media Foundation source reader (decodes parts as Windows players do), Core Audio session list (what plays into the virtual cable), `GetProcessAffinityMask` |

Dependencies (cpal, tauri, webview2-com, windows, ash and others) contain their own unsafe code;
this list covers code in this repository only.
