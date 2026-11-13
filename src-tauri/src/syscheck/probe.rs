//! What the machine is: cores and memory from `wb-machine` (affinity mask and job limit, never
//! `available_parallelism` or `GlobalMemoryStatusEx`), processor, instruction sets and graphics
//! adapters from the engines' own `info` (CPUID and DXGI are read there, so the app needs no second
//! probe), free disk space, audio inputs, the power source and the programs that hook windows.

use std::path::Path;

use wb_capability::{Disk, EngineStatus, Fingerprint, Gpu, GpuKind, GpuPrint, Hardware};
use wb_machine::Machine;

use crate::engines::{EngineRow, EngineState};

/// Programs known to inject a DLL into other programs' windows. WebView2 has crashed under RTSS's hook
/// (docs/evidence/phase-0/webview2-dump-analysis.txt), and these tools also disturb timing runs.
const INJECTOR_PROCESSES: &[(&str, &str)] = &[
    ("rtss.exe", "RivaTuner Statistics Server"),
    (
        "rtsshooksloader64.exe",
        "RivaTuner Statistics Server (hook loader)",
    ),
    (
        "rtsshooksloader.exe",
        "RivaTuner Statistics Server (hook loader)",
    ),
    (
        "encoderserver.exe",
        "RivaTuner Statistics Server (encoder server)",
    ),
    ("msiafterburner.exe", "MSI Afterburner"),
    ("latmon.exe", "LatencyMon"),
];
/// Module names (lower case, as a prefix) that mean a hook is loaded into a process.
const INJECTOR_MODULES: &[&str] = &["rtsshooks"];

/// Which injectors are running, from the process list and (for this process and the WebView2 helper
/// processes) the loaded modules. `modules_of(0)` is this process.
pub fn injectors_from(
    processes: &[(u32, String)],
    modules_of: impl Fn(u32) -> Vec<String>,
) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut add = |s: String| {
        if !found.contains(&s) {
            found.push(s);
        }
    };
    for (_, name) in processes {
        let lower = name.to_ascii_lowercase();
        if let Some((_, label)) = INJECTOR_PROCESSES.iter().find(|(n, _)| *n == lower) {
            add(format!("{label} ({name})"));
        }
    }
    let mut check_modules = |pid: u32, whose: &str| {
        for module in modules_of(pid) {
            let lower = module.to_ascii_lowercase();
            if INJECTOR_MODULES.iter().any(|p| lower.starts_with(p)) {
                add(format!("{module} is loaded into {whose}"));
            }
        }
    };
    check_modules(0, "this app");
    for (pid, name) in processes {
        if name.eq_ignore_ascii_case("msedgewebview2.exe") {
            check_modules(*pid, "the app's display (WebView2)");
        }
    }
    found
}

#[cfg(windows)]
pub fn running_injectors() -> Vec<String> {
    injectors_from(&super::win::running_processes(), super::win::loaded_modules)
}

#[cfg(not(windows))]
pub fn running_injectors() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn on_mains_power() -> Option<bool> {
    super::win::on_mains_power()
}

#[cfg(not(windows))]
fn on_mains_power() -> Option<bool> {
    None
}

#[cfg(windows)]
fn os_name() -> String {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let get = |v: &str| super::win::registry_text(KEY, v);
    let mut name = get("ProductName").unwrap_or_else(|| "Windows".into());
    // Windows 11 still writes "Windows 10" in ProductName; the build number tells them apart.
    let build = get("CurrentBuildNumber");
    if let Some(b) = build.as_deref().and_then(|b| b.parse::<u32>().ok())
        && b >= 22000
    {
        name = name.replace("Windows 10", "Windows 11");
    }
    let mut out = name;
    if let Some(v) = get("DisplayVersion") {
        out.push(' ');
        out.push_str(&v);
    }
    if let Some(b) = build {
        out.push_str(&format!(" (build {b})"));
    }
    out
}

#[cfg(not(windows))]
fn os_name() -> String {
    std::env::consts::OS.to_string()
}

fn vendor_of(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("rtx") {
        "NVIDIA".into()
    } else if n.contains("intel") {
        "Intel".into()
    } else if n.contains("amd") || n.contains("radeon") {
        "AMD".into()
    } else {
        "unknown".into()
    }
}

/// Graphics adapters from the first engine that lists any.
fn gpus_from(rows: &[EngineRow]) -> Vec<Gpu> {
    let Some(row) = rows
        .iter()
        .find(|r| matches!(r.state, EngineState::Ok) && r.devices.iter().any(|d| d.kind != "cpu"))
    else {
        return Vec::new();
    };
    row.devices
        .iter()
        .filter(|d| d.kind == "gpu" || d.kind == "igpu")
        .map(|d| {
            let integrated = d.kind == "igpu";
            let name = d
                .adapter_name
                .clone()
                .unwrap_or_else(|| d.description.clone());
            Gpu {
                index: d.index,
                vendor: vendor_of(&name),
                name,
                kind: if integrated {
                    GpuKind::Integrated
                } else {
                    GpuKind::Discrete
                },
                dedicated_bytes: if integrated { 0 } else { d.total_memory_bytes },
                // An integrated GPU's memory is system RAM; the check counts what it uses as RAM.
                local_bytes: d.total_memory_bytes,
                shared_bytes: d.shared_memory_bytes.unwrap_or(0),
                shared_assumed: false,
                driver: d.driver_version.clone(),
                vulkan_usable: d.backend.eq_ignore_ascii_case("vulkan"),
            }
        })
        .collect()
}

fn engine_status(rows: &[EngineRow]) -> Vec<EngineStatus> {
    rows.iter()
        .map(|r| EngineStatus {
            variant: r.variant.clone(),
            usable: matches!(r.state, EngineState::Ok),
            reason: match r.state {
                EngineState::Ok => None,
                EngineState::CpuMismatch => Some(r.message.clone().unwrap_or_else(|| {
                    "this processor lacks an instruction set the build needs".into()
                })),
                EngineState::Timeout => {
                    Some(r.message.clone().unwrap_or_else(|| "no answer".into()))
                }
                _ => Some(r.message.clone().unwrap_or_else(|| "failed".into())),
            },
            info_ms: Some(r.info_ms),
            devices: r.devices.iter().map(|d| d.name.clone()).collect(),
        })
        .collect()
}

/// Where the folders the check reports free space for are.
pub struct Roots<'a> {
    pub recordings: &'a Path,
    pub backup: Option<&'a Path>,
}

/// Fills in the hardware part of a profile.
pub fn hardware(
    machine: &Machine,
    rows: &[EngineRow],
    roots: &Roots<'_>,
) -> (Hardware, Vec<EngineStatus>) {
    let cpu = rows
        .iter()
        .find(|r| matches!(r.state, EngineState::Ok))
        .and_then(|r| r.cpu.as_ref());
    let features: Vec<String> = cpu.map(|c| c.features.clone()).unwrap_or_default();
    let has = |f: &str| features.iter().any(|x| x.eq_ignore_ascii_case(f));
    let mut disks = Vec::new();
    let mut disk = |role: &str, path: &Path| {
        if let Ok(free) = wb_recorder::sys::disk_free_bytes(path) {
            disks.push(Disk {
                role: role.into(),
                path: path.to_string_lossy().into_owned(),
                free_bytes: free,
            });
        }
    };
    disk("recordings", roots.recordings);
    if let Some(b) = roots.backup {
        disk("backup", b);
    }
    let hw = Hardware {
        os: os_name(),
        cpu_brand: cpu.map_or_else(|| "unknown processor".into(), |c| c.brand.clone()),
        logical_processors: machine.logical_processors as u32,
        physical_cores: machine.physical_cores as u32,
        avx2: has("avx2"),
        avx512: has("avx512f"),
        cpu_features: features,
        total_ram_bytes: machine.total_ram_bytes,
        available_ram_bytes: machine.available_ram_bytes,
        gpus: gpus_from(rows),
        disks,
        on_mains_power: on_mains_power(),
        audio_inputs: wb_recorder::capture::list_inputs().map_or(0, |v| v.len() as u32),
    };
    (hw, engine_status(rows))
}

/// The fingerprint of a hardware reading, the app version and the model manifest.
pub fn fingerprint(
    hw: &Hardware,
    app_version: &str,
    manifest_hash: &str,
    emulated: Option<&str>,
) -> Fingerprint {
    Fingerprint {
        os: hw.os.clone(),
        cpu_brand: hw.cpu_brand.clone(),
        logical_processors: hw.logical_processors,
        physical_cores: hw.physical_cores,
        total_ram_gib: (hw.total_ram_bytes as f64 / (1u64 << 30) as f64).round() as u32,
        gpus: hw
            .gpus
            .iter()
            .map(|g| GpuPrint {
                name: g.name.clone(),
                driver: g.driver.clone(),
            })
            .collect(),
        app_version: app_version.to_string(),
        manifest_hash: manifest_hash.to_string(),
        emulated: emulated.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(names: &[&str]) -> Vec<(u32, String)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (i as u32 + 10, (*n).to_string()))
            .collect()
    }

    #[test]
    fn rtss_and_afterburner_are_found_by_name_whatever_the_case() {
        let found = injectors_from(
            &procs(&["explorer.exe", "RTSS.exe", "MSIAfterburner.exe"]),
            |_| vec![],
        );
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("RivaTuner") && found[1].contains("Afterburner"));
    }

    #[test]
    fn a_hook_dll_in_the_app_or_its_display_is_reported() {
        let list = procs(&["msedgewebview2.exe", "chrome.exe"]);
        let found = injectors_from(&list, |pid| match pid {
            0 => vec!["kernel32.dll".into()],
            10 => vec!["RTSSHooks64.dll".into()],
            _ => vec!["RTSSHooks64.dll".into()],
        });
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("WebView2"));
    }

    #[test]
    fn a_clean_machine_reports_nothing() {
        assert!(injectors_from(&procs(&["explorer.exe"]), |_| vec!["ntdll.dll".into()]).is_empty());
    }

    #[test]
    fn vendors_come_from_the_adapter_name() {
        assert_eq!(vendor_of("NVIDIA GeForce RTX 4070 Laptop GPU"), "NVIDIA");
        assert_eq!(vendor_of("Intel(R) UHD Graphics"), "Intel");
        assert_eq!(vendor_of("AMD Radeon 780M Graphics"), "AMD");
        assert_eq!(vendor_of("Microsoft Basic Render Driver"), "unknown");
    }
}
