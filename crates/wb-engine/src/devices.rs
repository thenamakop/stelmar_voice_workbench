//! The device list `info` reports: one entry per physical GPU, the integrated ones marked, with
//! the driver version and shared memory from DXGI.
//!
//! The Vulkan loader on the Legion lists the Intel UHD GPU twice (indices 0 and 2) and DXGI lists
//! it twice as well (`docs/GATES.md`, Phase 1 -> 2). A device list that counts one GPU as two would
//! make the hybrid configuration run two GPU engines on one GPU. Entries are merged here, and each
//! kept entry remembers the position whisper.cpp needs (`gpu_ordinal`): its `gpu_device` setting
//! counts GPU-type devices in ggml's own order, duplicates included.

use wb_protocol::BackendDevice;

use crate::win::{Adapter, RawDevice, RawKind};

/// A kept device with the raw ordinal whisper.cpp needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub device: BackendDevice,
    /// Position among the GPU and integrated-GPU entries of ggml's list (what `gpu_device` means).
    pub gpu_ordinal: Option<u32>,
}

fn same_adapter(a: &RawDevice, b: &RawDevice) -> bool {
    if a.kind == RawKind::Cpu || b.kind == RawKind::Cpu {
        return false;
    }
    // The backend's own id for the adapter (a PCI bus id for Vulkan) settles it when both have one.
    if let (Some(x), Some(y)) = (&a.device_id, &b.device_id)
        && x == y
    {
        return true;
    }
    // The same description and memory size on integrated GPUs is the same adapter listed twice (a
    // second loader path or layer): a laptop has one integrated GPU. Two discrete cards of one model
    // keep their own bus ids and are not merged unless both ids are missing.
    let alike = a.description == b.description && a.total == b.total && a.kind == b.kind;
    if !alike {
        return false;
    }
    a.kind == RawKind::Igpu || (a.device_id.is_none() && b.device_id.is_none())
}

fn vendor_of(description: &str) -> Option<u32> {
    let d = description.to_ascii_lowercase();
    if d.contains("nvidia") || d.contains("geforce") || d.contains("quadro") || d.contains("rtx") {
        Some(0x10DE)
    } else if d.contains("intel") {
        Some(0x8086)
    } else if d.contains("amd") || d.contains("radeon") || d.contains("ati ") {
        Some(0x1002)
    } else {
        None
    }
}

/// The DXGI adapter for a GPU: same vendor (from the description), and the one whose memory is
/// closest to ggml's figure (dedicated for a discrete card, shared system memory for an integrated one).
fn adapter_for<'a>(d: &RawDevice, adapters: &'a [Adapter]) -> Option<&'a Adapter> {
    let vendor = vendor_of(&d.description).or_else(|| vendor_of(&d.name))?;
    let target = d.total;
    adapters
        .iter()
        .filter(|a| a.vendor_id == vendor && !a.software)
        .min_by_key(|a| {
            let mem = if d.kind == RawKind::Igpu {
                a.shared_system_memory
            } else {
                a.dedicated_video_memory
            };
            mem.abs_diff(target)
        })
}

/// De-duplicated device list: the CPU entries first, then one entry per physical GPU.
pub fn merge(raw: &[RawDevice], adapters: &[Adapter]) -> Vec<Kept> {
    let mut gpu_ordinals = std::collections::HashMap::new();
    let mut n = 0u32;
    for d in raw {
        if matches!(d.kind, RawKind::Gpu | RawKind::Igpu) {
            gpu_ordinals.insert(d.raw_index, n);
            n += 1;
        }
    }
    let mut kept: Vec<(RawDevice, u32)> = Vec::new();
    for d in raw {
        if let Some((_, dups)) = kept.iter_mut().find(|(k, _)| same_adapter(k, d)) {
            *dups += 1;
        } else {
            kept.push((d.clone(), 0));
        }
    }
    kept.into_iter()
        .enumerate()
        .map(|(index, (d, dups))| {
            let adapter = if matches!(d.kind, RawKind::Gpu | RawKind::Igpu) {
                adapter_for(&d, adapters)
            } else {
                None
            };
            let kind = match d.kind {
                RawKind::Cpu => "cpu",
                RawKind::Gpu => "gpu",
                RawKind::Igpu => "igpu",
                RawKind::Accel => "accel",
                RawKind::Other => "other",
            };
            Kept {
                gpu_ordinal: gpu_ordinals.get(&d.raw_index).copied(),
                device: BackendDevice {
                    index: index as u32,
                    backend: d
                        .name
                        .split(|c: char| c.is_ascii_digit())
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('_')
                        .to_string(),
                    name: d.name.clone(),
                    description: d.description.clone(),
                    kind: kind.into(),
                    total_memory_bytes: d.total,
                    free_memory_bytes: d.free,
                    driver_version: adapter.and_then(|a| a.driver_version.clone()),
                    duplicates_dropped: dups,
                    shared_memory_bytes: (d.kind == RawKind::Igpu)
                        .then(|| adapter.map_or(d.total, |a| a.shared_system_memory)),
                    adapter_name: adapter.map(|a| a.name.clone()),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(
        i: usize,
        name: &str,
        desc: &str,
        kind: RawKind,
        total: u64,
        id: Option<&str>,
    ) -> RawDevice {
        RawDevice {
            raw_index: i,
            name: name.into(),
            description: desc.into(),
            kind,
            free: total / 2,
            total,
            device_id: id.map(String::from),
        }
    }

    const GIB: u64 = 1 << 30;

    fn legion() -> Vec<RawDevice> {
        // The order the Legion's Vulkan loader gave in Phase 1: Intel (index 0), NVIDIA (1), Intel again (2).
        vec![
            raw(
                0,
                "CPU",
                "Intel(R) Core(TM) i9-14900HX",
                RawKind::Cpu,
                32 * GIB,
                None,
            ),
            raw(
                1,
                "Vulkan0",
                "Intel(R) Raptor Lake-S Mobile Graphics Controller",
                RawKind::Igpu,
                16 * GIB,
                Some("0000:00:02.0"),
            ),
            raw(
                2,
                "Vulkan1",
                "NVIDIA GeForce RTX 4070 Laptop GPU",
                RawKind::Gpu,
                8 * GIB,
                Some("0000:01:00.0"),
            ),
            raw(
                3,
                "Vulkan2",
                "Intel(R) Raptor Lake-S Mobile Graphics Controller",
                RawKind::Igpu,
                16 * GIB,
                Some("0000:00:02.0"),
            ),
        ]
    }

    fn adapters() -> Vec<Adapter> {
        let a = |name: &str, vendor: u32, ded: u64, shared: u64, drv: &str| Adapter {
            name: name.into(),
            vendor_id: vendor,
            device_id: 1,
            dedicated_video_memory: ded,
            shared_system_memory: shared,
            driver_version: Some(drv.into()),
            software: false,
        };
        vec![
            a(
                "Intel(R) UHD Graphics",
                0x8086,
                128 << 20,
                16 * GIB,
                "32.0.101.7092",
            ),
            a(
                "NVIDIA GeForce RTX 4070 Laptop GPU",
                0x10DE,
                8 * GIB,
                16 * GIB,
                "32.0.15.6214",
            ),
            a(
                "Intel(R) UHD Graphics",
                0x8086,
                128 << 20,
                16 * GIB,
                "32.0.101.7092",
            ),
            Adapter {
                software: true,
                ..a("Microsoft Basic Render Driver", 0x1414, 0, 0, "10.0.0.0")
            },
        ]
    }

    #[test]
    fn the_intel_gpu_listed_twice_is_one_device_and_the_nvidia_gpu_is_another() {
        let kept = merge(&legion(), &adapters());
        let gpus: Vec<&Kept> = kept.iter().filter(|k| k.device.kind != "cpu").collect();
        assert_eq!(gpus.len(), 2, "{gpus:?}");
        assert_eq!(gpus[0].device.kind, "igpu");
        assert_eq!(gpus[0].device.duplicates_dropped, 1);
        assert_eq!(gpus[1].device.kind, "gpu");
        assert_eq!(gpus[1].device.duplicates_dropped, 0);
    }

    #[test]
    fn indices_count_the_kept_devices_and_gpu_ordinals_count_ggmls_own_gpu_list() {
        let kept = merge(&legion(), &adapters());
        assert_eq!(
            kept.iter().map(|k| k.device.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // ggml's GPU-type list is [Intel, NVIDIA, Intel]: the Intel GPU is ordinal 0, the NVIDIA one 1.
        assert_eq!(
            kept.iter().map(|k| k.gpu_ordinal).collect::<Vec<_>>(),
            vec![None, Some(0), Some(1)]
        );
    }

    #[test]
    fn dxga_adapters_supply_the_driver_version_and_the_shared_memory() {
        let kept = merge(&legion(), &adapters());
        let igpu = &kept[1].device;
        assert_eq!(igpu.driver_version.as_deref(), Some("32.0.101.7092"));
        assert_eq!(igpu.shared_memory_bytes, Some(16 * GIB));
        assert_eq!(igpu.adapter_name.as_deref(), Some("Intel(R) UHD Graphics"));
        let nv = &kept[2].device;
        assert_eq!(nv.driver_version.as_deref(), Some("32.0.15.6214"));
        assert_eq!(
            nv.shared_memory_bytes, None,
            "a discrete card has its own memory"
        );
    }

    #[test]
    fn the_same_integrated_gpu_with_different_bus_ids_is_still_one_gpu() {
        // What wb-emulate printed on the Legion: two entries of one description with different ids.
        let mut raw = legion();
        raw[3].device_id = Some("0000:00:02.1".into());
        let kept = merge(&raw, &adapters());
        assert_eq!(kept.iter().filter(|k| k.device.kind == "igpu").count(), 1);
    }

    #[test]
    fn two_discrete_cards_of_one_model_stay_two_when_their_bus_ids_differ() {
        let raw = vec![
            raw(0, "CPU", "cpu", RawKind::Cpu, 32 * GIB, None),
            raw(
                1,
                "CUDA0",
                "NVIDIA RTX A2000",
                RawKind::Gpu,
                6 * GIB,
                Some("0000:01:00.0"),
            ),
            raw(
                2,
                "CUDA1",
                "NVIDIA RTX A2000",
                RawKind::Gpu,
                6 * GIB,
                Some("0000:02:00.0"),
            ),
        ];
        assert_eq!(
            merge(&raw, &[])
                .iter()
                .filter(|k| k.device.kind == "gpu")
                .count(),
            2
        );
    }

    #[test]
    fn without_any_gpu_only_the_cpu_is_listed() {
        let raw = vec![raw(
            0,
            "CPU",
            "Intel(R) Core(TM) i7",
            RawKind::Cpu,
            16 * GIB,
            None,
        )];
        let kept = merge(&raw, &[]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].device.kind, "cpu");
        assert_eq!(kept[0].gpu_ordinal, None);
    }

    #[test]
    fn a_device_missing_from_dxgi_is_still_listed_without_a_driver_version() {
        let kept = merge(&legion(), &[]);
        assert!(kept[1].device.driver_version.is_none());
        assert_eq!(
            kept[1].device.shared_memory_bytes,
            Some(16 * GIB),
            "falls back to ggml's figure"
        );
    }
}
