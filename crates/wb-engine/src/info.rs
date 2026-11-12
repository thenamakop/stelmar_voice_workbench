//! `wb-engine info`: what this build is, what the CPU can run, and every device found.

use wb_protocol::{InfoReport, WhisperCppInfo};

use crate::{cpu, devices, win};

/// The pinned whisper.cpp this engine was built from (compiled in by `build.rs`), with the version
/// the linked library itself reports.
pub fn whisper_cpp(library_version: &str) -> WhisperCppInfo {
    WhisperCppInfo {
        version: library_version.to_string(),
        commit: env!("WB_WHISPER_COMMIT").to_string(),
        tree_sha256: env!("WB_WHISPER_TREE_SHA256").to_string(),
    }
}

/// The pinned values only, with the tag in place of the library version: for the first output line,
/// which is printed before anything has touched whisper.cpp.
pub fn whisper_cpp_pinned() -> WhisperCppInfo {
    whisper_cpp(env!("WB_WHISPER_TAG"))
}

/// The build variant (`avx2`, `avx512`, `vulkan`, `cuda`, or `dev`).
pub fn variant() -> &'static str {
    env!("WB_ENGINE_VARIANT_NAME")
}

/// The CPU part of the report, from pure Rust: safe to compute before whisper.cpp is touched.
pub fn cpu_report() -> wb_protocol::CpuReport {
    let m = wb_machine::probe();
    cpu::report(
        variant(),
        cpu::brand(),
        m.logical_processors as u32,
        m.physical_cores as u32,
    )
}

/// The full report. Call only after the CPU check passed.
pub fn report() -> InfoReport {
    let m = wb_machine::probe();
    let raw = win::ggml_devices();
    let adapters = win::adapters();
    let kept = devices::merge(&raw, &adapters);
    let mut notes = Vec::new();
    let gpus = kept
        .iter()
        .filter(|k| k.device.kind == "gpu" || k.device.kind == "igpu")
        .count();
    if gpus == 0 {
        notes.push(format!(
            "No GPU device is usable by this build ({}). Adapters Windows lists: {}",
            variant(),
            if adapters.is_empty() {
                "none".to_string()
            } else {
                adapters
                    .iter()
                    .map(|a| {
                        format!(
                            "{} (driver {})",
                            a.name,
                            a.driver_version.as_deref().unwrap_or("unknown")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }
        ));
    }
    for k in &kept {
        if k.device.duplicates_dropped > 0 {
            notes.push(format!(
                "{} was listed {} times by the backend; counted once",
                k.device.description,
                k.device.duplicates_dropped + 1
            ));
        }
    }
    InfoReport {
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        variant: variant().to_string(),
        whisper_cpp: whisper_cpp(whisper_rs::get_whisper_version()),
        system_info: whisper_rs::print_system_info().to_string(),
        cpu: cpu_report(),
        backends: win::ggml_backends(),
        devices: kept.into_iter().map(|k| k.device).collect(),
        raw_device_count: raw.len() as u32,
        logical_processors_allowed: m.logical_processors as u32,
        available_ram_bytes: m.available_ram_bytes,
        notes,
    }
}
