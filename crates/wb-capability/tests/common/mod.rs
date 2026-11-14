//! Fixture profiles for the decision-rule tests. The speed numbers are copied from the Phase 2 speed
//! tables (docs/evidence/phase-2/speed-table-A|B|C.txt) so the fixtures behave like the machines they
//! are named after. Urdu has no row in the tables; the Hindi rows stand in for it.
#![allow(dead_code)]

use std::collections::BTreeMap;

use wb_capability::*;

pub const MIB: u64 = 1 << 20;

pub fn gib(x: f64) -> u64 {
    (x * GIB as f64) as u64
}

pub fn fingerprint(cpu: &str, ram_gib: u32, emulated: Option<&str>) -> Fingerprint {
    Fingerprint {
        os: "Windows 11".into(),
        cpu_brand: cpu.into(),
        logical_processors: 8,
        physical_cores: 4,
        total_ram_gib: ram_gib,
        gpus: vec![GpuPrint {
            name: "Fixture GPU".into(),
            driver: Some("1.0".into()),
        }],
        app_version: "0.1.0".into(),
        manifest_hash: "fixture".into(),
        emulated: emulated.map(str::to_string),
    }
}

pub fn igpu(index: u32) -> Gpu {
    Gpu {
        index,
        name: "Intel(R) UHD Graphics".into(),
        vendor: "Intel".into(),
        kind: GpuKind::Integrated,
        dedicated_bytes: 0,
        local_bytes: gib(2.1),
        shared_bytes: 0,
        shared_assumed: false,
        driver: Some("32.0.101.0".into()),
        vulkan_usable: true,
    }
}

pub fn rtx(index: u32) -> Gpu {
    Gpu {
        index,
        name: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
        vendor: "NVIDIA".into(),
        kind: GpuKind::Discrete,
        dedicated_bytes: gib(8.0),
        local_bytes: gib(8.0),
        shared_bytes: 0,
        shared_assumed: false,
        driver: Some("560.0".into()),
        vulkan_usable: true,
    }
}

pub fn hardware(
    physical: u32,
    logical: u32,
    total_gib: f64,
    available_gib: f64,
    gpus: Vec<Gpu>,
) -> Hardware {
    Hardware {
        os: "Windows 11".into(),
        cpu_brand: "Fixture CPU".into(),
        logical_processors: logical,
        physical_cores: physical,
        cpu_features: vec![
            "avx".into(),
            "avx2".into(),
            "fma".into(),
            "f16c".into(),
            "bmi2".into(),
        ],
        avx2: true,
        avx512: false,
        total_ram_bytes: gib(total_gib),
        available_ram_bytes: gib(available_gib),
        gpus,
        disks: vec![],
        on_mains_power: Some(true),
        audio_inputs: 2,
    }
}

pub fn engines_ok() -> Vec<EngineStatus> {
    vec![
        EngineStatus {
            variant: "avx2".into(),
            usable: true,
            reason: None,
            info_ms: Some(300),
            devices: vec!["cpu".into()],
        },
        EngineStatus {
            variant: "avx512".into(),
            usable: false,
            reason: Some("this CPU has no AVX-512 (exit code 3)".into()),
            info_ms: Some(40),
            devices: vec![],
        },
        EngineStatus {
            variant: "vulkan".into(),
            usable: true,
            reason: None,
            info_ms: Some(900),
            devices: vec!["gpu".into()],
        },
    ]
}

pub fn profile(emulated: Option<&str>, hw: Hardware) -> Profile {
    Profile {
        schema_version: SCHEMA_VERSION,
        app_version: "0.1.0".into(),
        created_at: "2026-11-13T20:00:00+05:30".into(),
        emulated: emulated.map(str::to_string),
        fingerprint: fingerprint("Fixture CPU", (hw.total_ram_bytes / GIB) as u32, emulated),
        check: CheckInfo {
            kind: CheckKind::Quick,
            partial: false,
            started_at: "2026-11-13T20:00:00+05:30".into(),
            duration_s: 400.0,
            bar_s: 480.0,
            excerpt_seconds: 30.0,
        },
        hardware: hw,
        engines: engines_ok(),
        measurements: vec![],
        hybrid: None,
        recorder_safety: vec![],
        injectors: vec![],
        skipped: vec![],
    }
}

/// One rolling or near-live cell. `commit_gib` is the engine's peak commit; `local_gib` the GPU's
/// DXGI local memory (None on the CPU).
#[allow(clippy::too_many_arguments)]
pub fn cell(
    model: &str,
    lang: Lang,
    mode: Mode,
    config: DeviceConfig,
    rtf: f64,
    commit_gib: f64,
    local_gib: Option<f64>,
    truncated: u32,
) -> Measurement {
    let build = if matches!(config, DeviceConfig::Cpu) {
        "avx2"
    } else {
        "vulkan"
    };
    Measurement {
        model: model.into(),
        lang,
        mode,
        config,
        engine_build: build.into(),
        device_name: "fixture".into(),
        threads: 2,
        method: "fixture (numbers from the Phase 2 speed tables)".into(),
        clip_seconds: 30.0,
        rtf,
        warmup_rtf: Some(rtf * 1.1),
        load_s: 1.0,
        peak_working_set_bytes: gib(commit_gib * 0.8),
        peak_commit_bytes: gib(commit_gib),
        gpu_local_bytes: local_gib.map(gib),
        truncated_chunks: truncated,
        live_standin: false,
    }
}

pub fn rolling(
    model: &str,
    lang: Lang,
    config: DeviceConfig,
    rtf: f64,
    commit: f64,
    local: Option<f64>,
) -> Measurement {
    cell(model, lang, Mode::Rolling, config, rtf, commit, local, 0)
}

pub fn near_live(model: &str, lang: Lang, rtf: f64, truncated: u32) -> Measurement {
    cell(
        model,
        lang,
        Mode::NearLive,
        DeviceConfig::Cpu,
        rtf,
        1.2,
        None,
        truncated,
    )
}

pub fn standin(model: &str, lang: Lang, gpu: u32, rtf: f64) -> Measurement {
    let mut m = cell(
        model,
        lang,
        Mode::NearLive,
        DeviceConfig::Gpu { index: gpu },
        rtf,
        1.5,
        Some(1.0),
        0,
    );
    m.live_standin = true;
    m
}

fn ranked(model: &str, bytes: u64, unverified: bool, validated: bool) -> Ranked {
    Ranked {
        model: model.into(),
        bytes,
        quality_unverified: unverified,
        validated,
    }
}

/// The test fixture ranking (NOT the manifest's): hi prefers turbo, then medium, then small.
pub fn rankings(near_live_validated: bool) -> Rankings {
    let medium = 1_533_763_059;
    let turbo = 574_041_195;
    let small = 487_601_967;
    let large = 1_081_140_203;
    let mut r = Rankings {
        provisional: true,
        ..Rankings::default()
    };
    r.rolling.insert(
        Lang::En,
        vec![
            ranked("medium", medium, false, false),
            ranked("large-v3-turbo-q5_0", turbo, true, false),
            ranked("small", small, true, false),
        ],
    );
    r.rolling.insert(
        Lang::Hi,
        vec![
            ranked("large-v3-turbo-q5_0", turbo, true, false),
            ranked("medium", medium, false, false),
            ranked("small", small, true, false),
        ],
    );
    r.rolling.insert(
        Lang::Ur,
        vec![
            ranked("medium", medium, false, false),
            ranked("small", small, true, false),
        ],
    );
    for l in Lang::ALL {
        r.night
            .insert(l, vec![ranked("large-v3-q5_0", large, true, false)]);
    }
    r.near_live.insert(
        Lang::En,
        vec![ranked("small", small, true, near_live_validated)],
    );
    r.near_live.insert(
        Lang::Hi,
        vec![ranked("small", small, true, near_live_validated)],
    );
    // The same flag stands for "Phase 5 checked these models for accuracy" (`validated_roles`).
    r.live.insert(
        Lang::En,
        vec![ranked(
            "large-v3-turbo-q5_0",
            turbo,
            true,
            near_live_validated,
        )],
    );
    r.live.insert(
        Lang::Hi,
        vec![ranked(
            "large-v3-turbo-q5_0",
            turbo,
            true,
            near_live_validated,
        )],
    );
    r.live.insert(
        Lang::Ur,
        vec![ranked("large-v3-q5_0", large, true, near_live_validated)],
    );
    r
}

fn gpu0(index: u32) -> DeviceConfig {
    DeviceConfig::Gpu { index }
}

/// A complete check also ran the recorder safety test for the configurations it chose between.
fn with_safety(mut p: Profile, configs: &[DeviceConfig]) -> Profile {
    for c in configs {
        p.recorder_safety.push(RecorderSafety {
            config: *c,
            duration_s: 120.0,
            overruns: 0,
            driver_xruns: 0,
            passed: true,
        });
    }
    p
}

/// emu-low (speed table A): 4 efficiency cores, the Intel UHD only, an 11 GiB job reported as 16 GB.
pub fn emu_low() -> Profile {
    let mut p = profile(Some("emu-low"), hardware(4, 4, 16.0, 11.0, vec![igpu(0)]));
    let g = gpu0(0);
    let c = DeviceConfig::Cpu;
    let u = Some(0.88);
    p.measurements = vec![
        // Vulkan on the UHD, 2 CPU threads (table A)
        rolling("medium", Lang::En, g, 0.354, 2.74, Some(2.11)),
        rolling("medium", Lang::Hi, g, 0.929, 2.75, Some(2.10)),
        rolling("medium", Lang::Ur, g, 0.929, 2.75, Some(2.10)),
        rolling("large-v3-turbo-q5_0", Lang::En, g, 0.359, 1.51, u),
        rolling("large-v3-turbo-q5_0", Lang::Hi, g, 0.734, 1.49, u),
        rolling("small", Lang::En, g, 0.139, 1.44, Some(0.82)),
        rolling("small", Lang::Hi, g, 0.423, 1.43, Some(0.82)),
        rolling("small", Lang::Ur, g, 0.423, 1.43, Some(0.82)),
        rolling("large-v3-q5_0", Lang::Hi, g, 1.532, 2.72, Some(2.03)),
        // CPU avx2, 4 threads (table A)
        rolling("small", Lang::En, c, 0.948, 1.32, None),
        rolling("small", Lang::Hi, c, 1.629, 1.33, None),
        rolling("medium", Lang::Hi, c, 5.203, 2.62, None),
        // near-live small, 2 threads
        near_live("small", Lang::En, 1.641, 0),
        near_live("small", Lang::Hi, 2.350, 1),
    ];
    with_safety(p, &[g, DeviceConfig::Cpu])
}

/// emu-mid (speed table B) with a hybrid cell and a passing recorder safety run.
pub fn emu_mid_hybrid() -> Profile {
    let mut p = profile(Some("emu-mid"), hardware(10, 12, 16.0, 11.0, vec![igpu(0)]));
    let g = gpu0(0);
    let c = DeviceConfig::Cpu;
    p.measurements = vec![
        rolling("medium", Lang::En, g, 0.347, 2.75, Some(2.11)),
        rolling("medium", Lang::Hi, g, 0.866, 2.75, Some(2.10)),
        rolling("large-v3-turbo-q5_0", Lang::En, g, 0.361, 1.51, Some(0.88)),
        rolling("large-v3-turbo-q5_0", Lang::Hi, g, 0.659, 1.49, Some(0.88)),
        rolling("small", Lang::Ur, g, 0.360, 1.43, Some(0.82)),
        rolling("large-v3-turbo-q5_0", Lang::Hi, c, 1.839, 1.38, None),
        rolling("small", Lang::Hi, c, 0.671, 1.33, None),
        near_live("small", Lang::En, 0.391, 0),
        near_live("small", Lang::Hi, 0.662, 2),
    ];
    p.hybrid = Some(HybridMeasurement {
        gpu_alone_rtf: None,
        model: "large-v3-turbo-q5_0".into(),
        lang: Lang::Hi,
        gpu: 0,
        cpu_threads: 8,
        gpu_rtf_concurrent: 0.70,
        cpu_rtf_concurrent: 2.30,
        cpu_peak_commit_bytes: gib(1.38),
        gpu_peak_commit_bytes: gib(1.49),
        gpu_local_bytes: Some(gib(0.88)),
        method: "fixture".into(),
    });
    p.recorder_safety = vec![
        RecorderSafety {
            config: DeviceConfig::Hybrid { gpu: 0 },
            duration_s: 120.0,
            overruns: 0,
            driver_xruns: 0,
            passed: true,
        },
        RecorderSafety {
            config: g,
            duration_s: 120.0,
            overruns: 0,
            driver_xruns: 0,
            passed: true,
        },
    ];
    p
}

/// The Legion, native (speed table C): the RTX 4070 as device 1 and the UHD as device 0, 32 GB.
pub fn legion_native() -> Profile {
    let mut p = profile(None, hardware(24, 32, 32.0, 24.0, vec![igpu(0), rtx(1)]));
    let g = gpu0(1);
    let c = DeviceConfig::Cpu;
    let r = Some(0.9);
    p.measurements = vec![
        rolling("medium", Lang::En, g, 0.047, 1.17, Some(2.26)),
        rolling("medium", Lang::Hi, g, 0.188, 1.17, Some(2.14)),
        rolling("medium", Lang::Ur, g, 0.188, 1.17, Some(2.14)),
        rolling("large-v3-turbo-q5_0", Lang::En, g, 0.042, 0.62, r),
        rolling("large-v3-turbo-q5_0", Lang::Hi, g, 0.114, 0.62, r),
        rolling("large-v3-q5_0", Lang::En, g, 0.061, 1.08, Some(2.05)),
        rolling("large-v3-q5_0", Lang::Hi, g, 0.245, 1.08, Some(2.10)),
        rolling("large-v3-q5_0", Lang::Ur, g, 0.245, 1.08, Some(2.10)),
        rolling("hinglish-q5_0", Lang::Hi, g, 0.250, 1.08, Some(2.10)),
        rolling("large-v3-turbo-q5_0", Lang::Hi, c, 1.363, 0.91, None),
        near_live("small", Lang::En, 0.248, 0),
        near_live("small", Lang::Hi, 0.497, 2),
        standin("large-v3-turbo-q5_0", Lang::En, 1, 0.05),
        standin("large-v3-turbo-q5_0", Lang::Hi, 1, 0.12),
        standin("large-v3-q5_0", Lang::Ur, 1, 0.14),
    ];
    with_safety(p, &[g, c])
}

/// A laptop with a strong integrated GPU and a quick CPU, near-live small models quick and clean.
pub fn strong_igpu_laptop() -> Profile {
    let mut p = profile(None, hardware(8, 12, 16.0, 11.0, vec![igpu(0)]));
    let g = gpu0(0);
    p.measurements = vec![
        rolling("medium", Lang::En, g, 0.20, 2.7, Some(2.1)),
        rolling("large-v3-turbo-q5_0", Lang::Hi, g, 0.35, 1.5, Some(0.9)),
        rolling("small", Lang::Ur, g, 0.20, 1.4, Some(0.8)),
        near_live("small", Lang::En, 0.18, 0),
        near_live("small", Lang::Hi, 0.30, 0),
    ];
    with_safety(p, &[g])
}

/// The machine with no usable GPU at all, emulated like emu-low: CPU only.
pub fn cpu_only_weak() -> Profile {
    let mut p = profile(Some("emu-low"), hardware(4, 4, 16.0, 11.0, vec![]));
    p.engines.retain(|e| e.variant != "vulkan");
    let c = DeviceConfig::Cpu;
    p.measurements = vec![
        rolling("medium", Lang::En, c, 3.002, 2.61, None),
        rolling("small", Lang::En, c, 0.948, 1.32, None),
        rolling("large-v3-turbo-q5_0", Lang::Hi, c, 3.894, 1.38, None),
        rolling("medium", Lang::Hi, c, 5.203, 2.62, None),
        rolling("small", Lang::Hi, c, 1.629, 1.33, None),
        rolling("medium", Lang::Ur, c, 5.203, 2.62, None),
        rolling("small", Lang::Ur, c, 1.629, 1.33, None),
        near_live("small", Lang::En, 1.641, 0),
        near_live("small", Lang::Hi, 2.350, 1),
    ];
    with_safety(p, &[c])
}

/// An integrated GPU slower than the CPU, and a hybrid cell that measured slower than the CPU alone
/// (the two engines fight over memory bandwidth): hybrid must not be chosen.
pub fn slow_igpu() -> Profile {
    let mut p = profile(None, hardware(6, 12, 16.0, 11.0, vec![igpu(0)]));
    let g = gpu0(0);
    let c = DeviceConfig::Cpu;
    p.measurements = vec![
        rolling("medium", Lang::En, c, 0.50, 2.6, None),
        rolling("medium", Lang::En, g, 0.80, 2.7, Some(2.1)),
        rolling("large-v3-turbo-q5_0", Lang::Hi, c, 0.55, 1.4, None),
        rolling("large-v3-turbo-q5_0", Lang::Hi, g, 0.90, 1.5, Some(0.9)),
        rolling("small", Lang::Ur, c, 0.30, 1.3, None),
        rolling("small", Lang::Ur, g, 0.60, 1.4, Some(0.8)),
    ];
    p.hybrid = Some(HybridMeasurement {
        gpu_alone_rtf: None,
        model: "large-v3-turbo-q5_0".into(),
        lang: Lang::Hi,
        gpu: 0,
        cpu_threads: 4,
        gpu_rtf_concurrent: 2.0,
        cpu_rtf_concurrent: 1.4,
        cpu_peak_commit_bytes: gib(1.4),
        gpu_peak_commit_bytes: gib(1.5),
        gpu_local_bytes: Some(gib(0.9)),
        method: "fixture".into(),
    });
    p.recorder_safety = vec![RecorderSafety {
        config: DeviceConfig::Hybrid { gpu: 0 },
        duration_s: 120.0,
        overruns: 0,
        driver_xruns: 0,
        passed: true,
    }];
    with_safety(p, &[c, g])
}

/// An old PC without AVX2: record only.
pub fn old_pc() -> Profile {
    let mut hw = hardware(2, 4, 8.0, 5.0, vec![]);
    hw.avx2 = false;
    hw.cpu_features = vec!["avx".into()];
    let mut p = profile(None, hw);
    p.engines = vec![EngineStatus {
        variant: "avx2".into(),
        usable: false,
        reason: Some("this CPU lacks avx2 (exit code 3)".into()),
        info_ms: Some(30),
        devices: vec![],
    }];
    p
}

/// A machine whose avx512 engine crashed at `info`; the avx2 build works.
pub fn avx512_crash() -> Profile {
    let mut p = profile(None, hardware(6, 12, 16.0, 11.0, vec![]));
    p.hardware.avx512 = true;
    p.engines = vec![
        EngineStatus {
            variant: "avx512".into(),
            usable: false,
            reason: Some("the engine crashed at start (exit code 0xC0000005)".into()),
            info_ms: Some(20_000),
            devices: vec![],
        },
        EngineStatus {
            variant: "avx2".into(),
            usable: true,
            reason: None,
            info_ms: Some(250),
            devices: vec!["cpu".into()],
        },
    ];
    let c = DeviceConfig::Cpu;
    p.measurements = vec![
        rolling("medium", Lang::En, c, 0.60, 2.6, None),
        rolling("large-v3-turbo-q5_0", Lang::Hi, c, 0.70, 1.4, None),
        rolling("medium", Lang::Ur, c, 0.80, 2.6, None),
    ];
    p
}

pub fn decide_with(p: &Profile, near_live_validated: bool) -> Decision {
    decide(p, &rankings(near_live_validated), &Options::default())
}

pub fn cap(p: Profile, near_live_validated: bool) -> Capability {
    assemble(
        p,
        &rankings(near_live_validated),
        &Options::default(),
        vec![],
    )
}

pub fn by_lang(d: &Decision) -> &BTreeMap<Lang, LangDecision> {
    &d.languages
}
