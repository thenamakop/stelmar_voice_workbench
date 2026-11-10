//! CPU feature checks, made before anything touches whisper.cpp.
//!
//! The engine is built with fixed instruction-set flags, never the build machine's own
//! (`GGML_NATIVE=OFF`), so a laptop that lacks an instruction set the build uses must find out
//! here, with a clear message and exit code 3, and not by an illegal-instruction crash inside ggml.

use wb_protocol::CpuReport;

/// What each build variant needs.
pub fn required_features(variant: &str) -> Vec<&'static str> {
    let mut v = vec!["avx", "avx2", "fma", "f16c"];
    if variant == "avx512" {
        v.extend(["avx512f", "avx512cd", "avx512vl", "avx512dq", "avx512bw"]);
    }
    v
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn has(name: &str) -> bool {
    match name {
        "avx" => std::arch::is_x86_feature_detected!("avx"),
        "avx2" => std::arch::is_x86_feature_detected!("avx2"),
        "fma" => std::arch::is_x86_feature_detected!("fma"),
        "f16c" => std::arch::is_x86_feature_detected!("f16c"),
        "avx512f" => std::arch::is_x86_feature_detected!("avx512f"),
        "avx512cd" => std::arch::is_x86_feature_detected!("avx512cd"),
        "avx512vl" => std::arch::is_x86_feature_detected!("avx512vl"),
        "avx512dq" => std::arch::is_x86_feature_detected!("avx512dq"),
        "avx512bw" => std::arch::is_x86_feature_detected!("avx512bw"),
        "avx512vnni" => std::arch::is_x86_feature_detected!("avx512vnni"),
        "avx512bf16" => std::arch::is_x86_feature_detected!("avx512bf16"),
        "sse4.2" => std::arch::is_x86_feature_detected!("sse4.2"),
        _ => false,
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn has(_name: &str) -> bool {
    false
}

const REPORTED: &[&str] = &[
    "sse4.2",
    "avx",
    "avx2",
    "fma",
    "f16c",
    "avx512f",
    "avx512cd",
    "avx512vl",
    "avx512dq",
    "avx512bw",
    "avx512vnni",
    "avx512bf16",
];

/// The CPU report for `info`, with what the variant needs and what is missing.
pub fn report(variant: &str, brand: String, logical: u32, physical: u32) -> CpuReport {
    let required = required_features(variant);
    let features: Vec<String> = REPORTED
        .iter()
        .filter(|f| has(f))
        .map(|f| (*f).to_string())
        .collect();
    let missing: Vec<String> = required
        .iter()
        .filter(|f| !has(f))
        .map(|f| (*f).to_string())
        .collect();
    CpuReport {
        brand,
        logical_processors: logical,
        physical_cores: physical,
        features,
        required_features: required.iter().map(|f| (*f).to_string()).collect(),
        missing_features: missing,
    }
}

/// The CPU name Windows reports (`PROCESSOR_IDENTIFIER`: family, model and vendor).
pub fn brand() -> String {
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_avx512_variant_needs_more_than_the_avx2_variant() {
        let a = required_features("avx2");
        let b = required_features("avx512");
        assert!(b.len() > a.len());
        assert!(a.iter().all(|f| b.contains(f)));
        assert!(!a.contains(&"avx512f") && b.contains(&"avx512f"));
        assert_eq!(required_features("vulkan"), a);
        assert_eq!(required_features("cuda"), a);
    }

    #[test]
    fn missing_features_are_the_required_ones_the_cpu_lacks() {
        let r = report("avx512", "test".into(), 4, 4);
        for m in &r.missing_features {
            assert!(r.required_features.contains(m));
            assert!(!r.features.contains(m));
        }
        assert!(r.features.iter().all(|f| !r.missing_features.contains(f)));
    }

    #[test]
    fn this_machine_runs_the_baseline_build() {
        // The portable baseline (AVX2, FMA, F16C) is the minimum the project supports (PLAN section 11).
        let r = report("avx2", "test".into(), 1, 1);
        assert!(r.missing_features.is_empty(), "{:?}", r.missing_features);
    }
}
