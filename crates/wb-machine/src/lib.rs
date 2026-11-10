//! What this process may really use.
//!
//! Inside a Windows Job Object (the emulation job of `wb-emulate`, or a laptop's own limits) a
//! process still sees the host: `std::thread::available_parallelism()` and the async runtime
//! report all 32 processors of the Legion under `emu-low`, which has 4 in its affinity mask, and
//! `GlobalMemoryStatusEx` reports the host's 32 GB, not the job's 11 GB. Thread counts and memory
//! rules must follow the mask and the job limit (AGENTS.md: threads and the 3 GB / 2 GB memory
//! rules come from what the machine really allows; `docs/GATES.md`, Phase 1 -> 2).
//!
//! The decisions here are plain functions of [`Inputs`], tested without Windows; `probe` fills the
//! inputs from the process itself.

use serde::Serialize;

#[cfg(windows)]
mod win;

/// Environment variables `wb-emulate` sets for the process it runs (development only).
pub const ENV_EMULATE: &str = "WB_EMULATE";
pub const ENV_REPORTED_RAM: &str = "WB_EMULATE_REPORTED_RAM_BYTES";
pub const ENV_JOB_MEMORY: &str = "WB_EMULATE_JOB_MEMORY_BYTES";

/// What a process may use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Machine {
    /// Logical processors in the affinity mask.
    pub logical_processors: usize,
    /// Physical cores that have at least one logical processor in the mask.
    pub physical_cores: usize,
    pub affinity_mask: u64,
    /// What `std::thread::available_parallelism()` says, for comparison (the host under a job).
    pub std_available_parallelism: usize,
    /// Total RAM as the machine reports it: 16 GB under emulation, the host's otherwise.
    pub total_ram_bytes: u64,
    pub host_total_ram_bytes: u64,
    /// RAM this process tree can still use: the smaller of the host's available memory and the
    /// job limit less what the job already uses.
    pub available_ram_bytes: u64,
    pub job_memory_limit_bytes: Option<u64>,
    pub job_commit_used_bytes: Option<u64>,
    /// The emulation profile (`emu-low`, `emu-mid`, `native`) when run under `wb-emulate`.
    pub emulated: Option<String>,
}

/// The raw facts [`Machine::from_inputs`] decides from.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub affinity_mask: u64,
    /// One mask per physical core (logical processors of processor group 0).
    pub core_masks: Vec<u64>,
    pub std_available_parallelism: usize,
    pub host_total_ram: u64,
    pub host_available_ram: u64,
    /// The commit limit of the job the process is in, if any.
    pub job_limit: Option<u64>,
    /// Commit of the processes in that job.
    pub job_used: Option<u64>,
    pub env_emulated: Option<String>,
    pub env_reported_ram: Option<u64>,
    pub env_job_memory: Option<u64>,
}

impl Machine {
    pub fn from_inputs(i: &Inputs) -> Self {
        let logical = i.affinity_mask.count_ones() as usize;
        let physical = i
            .core_masks
            .iter()
            .filter(|m| **m & i.affinity_mask != 0)
            .count();
        // Without core records assume no SMT, so threads are not over-counted as cores.
        let physical_cores = if physical == 0 { logical } else { physical };
        // The nearest limit wins: the job's own, and the emulation's (set in the environment
        // because an engine in a nested job does not see its parent job's limit).
        let limit = match (i.job_limit, i.env_job_memory) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let from_job = limit.map(|l| l.saturating_sub(i.job_used.unwrap_or(0)));
        let available = from_job.map_or(i.host_available_ram, |j| j.min(i.host_available_ram));
        Self {
            logical_processors: logical.max(1),
            physical_cores: physical_cores.max(1),
            affinity_mask: i.affinity_mask,
            std_available_parallelism: i.std_available_parallelism,
            total_ram_bytes: i.env_reported_ram.unwrap_or(i.host_total_ram),
            host_total_ram_bytes: i.host_total_ram,
            available_ram_bytes: available,
            job_memory_limit_bytes: limit,
            job_commit_used_bytes: i.job_used,
            emulated: i.env_emulated.clone(),
        }
    }

    /// Threads for post-processing engines in total: physical cores minus one (PLAN section 3),
    /// at least one.
    pub fn batch_threads(&self) -> usize {
        self.physical_cores.saturating_sub(1).max(1)
    }

    /// Threads for near-live and live engines: half the logical processors (PLAN section 3).
    pub fn live_threads(&self) -> usize {
        (self.logical_processors / 2).max(1)
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.trim().parse().ok()
}

/// Reads the machine view of the running process.
pub fn probe() -> Machine {
    let std_parallelism = std::thread::available_parallelism().map_or(1, |n| n.get());
    #[allow(unused_mut)]
    let mut inputs = Inputs {
        std_available_parallelism: std_parallelism,
        env_emulated: std::env::var(ENV_EMULATE).ok().filter(|s| !s.is_empty()),
        env_reported_ram: env_u64(ENV_REPORTED_RAM),
        env_job_memory: env_u64(ENV_JOB_MEMORY),
        ..Inputs::default()
    };
    #[cfg(windows)]
    {
        inputs.affinity_mask = win::process_affinity().unwrap_or(0);
        if let Some(records) = win::processor_core_records() {
            inputs.core_masks = core_masks(&records);
        }
        if let Some((total, avail)) = win::host_memory() {
            inputs.host_total_ram = total;
            inputs.host_available_ram = avail;
        }
        inputs.job_limit = win::job_memory_limit();
        inputs.job_used = win::job_commit_used();
    }
    #[cfg(not(windows))]
    {
        inputs.affinity_mask = if std_parallelism >= 64 {
            u64::MAX
        } else {
            (1u64 << std_parallelism) - 1
        };
    }
    Machine::from_inputs(&inputs)
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(o)?, *b.get(o + 1)?]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    let s = b.get(o..o + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    let s = b.get(o..o + 8)?;
    Some(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

/// One mask per physical core, from SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX records
/// (`RelationProcessorCore`, x64 layout): Relationship u32 @0, Size u32 @4, then
/// PROCESSOR_RELATIONSHIP @8 { Flags u8, EfficiencyClass u8, Reserved [u8; 20], GroupCount u16
/// @30, GroupMask @32: [{ Mask u64, Group u16, Reserved [u16; 3] }] (16 bytes each) }. Cores in
/// processor groups other than 0 are skipped (the affinity mask is for one group).
pub fn core_masks(bytes: &[u8]) -> Vec<u64> {
    const RELATION_PROCESSOR_CORE: u32 = 0;
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let (Some(rel), Some(size)) = (u32_at(bytes, off), u32_at(bytes, off + 4)) else {
            break;
        };
        if size == 0 {
            break;
        }
        if rel == RELATION_PROCESSOR_CORE {
            let groups = u16_at(bytes, off + 30).unwrap_or(0);
            let mut mask = 0u64;
            for g in 0..usize::from(groups) {
                let base = off + 32 + g * 16;
                if let (Some(m), Some(group)) = (u64_at(bytes, base), u16_at(bytes, base + 8)) {
                    if group == 0 {
                        mask |= m;
                    }
                }
            }
            if mask != 0 {
                out.push(mask);
            }
        }
        off += size as usize;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn base() -> Inputs {
        Inputs {
            affinity_mask: 0xFFFF_FFFF,
            // 8 performance cores with SMT then 16 efficiency cores.
            core_masks: (0..8)
                .map(|c| 0b11u64 << (2 * c))
                .chain((0..16).map(|c| 1u64 << (16 + c)))
                .collect(),
            std_available_parallelism: 32,
            host_total_ram: 32 * GIB,
            host_available_ram: 20 * GIB,
            ..Inputs::default()
        }
    }

    #[test]
    fn the_whole_machine_has_32_threads_and_24_cores() {
        let m = Machine::from_inputs(&base());
        assert_eq!((m.logical_processors, m.physical_cores), (32, 24));
        assert_eq!(m.total_ram_bytes, 32 * GIB);
        assert_eq!(m.available_ram_bytes, 20 * GIB);
        assert!(m.emulated.is_none());
    }

    #[test]
    fn emu_low_sees_four_processors_though_std_says_32() {
        // 4 efficiency cores: logical processors 16 to 19.
        let mut i = base();
        i.affinity_mask = 0xF0000;
        i.job_limit = Some(11 * GIB);
        i.job_used = Some(GIB);
        i.env_emulated = Some("emu-low".into());
        i.env_reported_ram = Some(16 * GIB);
        let m = Machine::from_inputs(&i);
        assert_eq!(m.logical_processors, 4);
        assert_eq!(m.physical_cores, 4);
        assert_eq!(m.std_available_parallelism, 32);
        assert_eq!(m.total_ram_bytes, 16 * GIB);
        assert_eq!(m.host_total_ram_bytes, 32 * GIB);
        // The job's 11 GB less the 1 GB it already uses, not the host's 20 GB free.
        assert_eq!(m.available_ram_bytes, 10 * GIB);
        assert_eq!(m.batch_threads(), 3);
        assert_eq!(m.live_threads(), 2);
        assert_eq!(m.emulated.as_deref(), Some("emu-low"));
    }

    #[test]
    fn emu_mid_counts_cores_not_threads() {
        // 2 performance cores with SMT (logical 0 to 3) plus 8 efficiency cores (16 to 23).
        let mut i = base();
        i.affinity_mask = 0b1111 | (0xFF << 16);
        let m = Machine::from_inputs(&i);
        assert_eq!((m.logical_processors, m.physical_cores), (12, 10));
        assert_eq!(m.batch_threads(), 9);
        assert_eq!(m.live_threads(), 6);
    }

    #[test]
    fn a_nested_job_takes_the_emulation_limit_from_the_environment() {
        // An engine in its own job inside the emulation job sees no limit of its own.
        let mut i = base();
        i.job_limit = None;
        i.env_job_memory = Some(11 * GIB);
        assert_eq!(Machine::from_inputs(&i).available_ram_bytes, 11 * GIB);
        // Two limits: the smaller wins.
        i.job_limit = Some(8 * GIB);
        let m = Machine::from_inputs(&i);
        assert_eq!(m.job_memory_limit_bytes, Some(8 * GIB));
        assert_eq!(m.available_ram_bytes, 8 * GIB);
    }

    #[test]
    fn available_memory_is_never_more_than_the_host_has_free() {
        let mut i = base();
        i.job_limit = Some(11 * GIB);
        i.host_available_ram = 3 * GIB;
        assert_eq!(Machine::from_inputs(&i).available_ram_bytes, 3 * GIB);
        // A job that uses more than its limit has nothing left.
        i.host_available_ram = 20 * GIB;
        i.job_used = Some(12 * GIB);
        assert_eq!(Machine::from_inputs(&i).available_ram_bytes, 0);
    }

    #[test]
    fn without_core_records_threads_are_not_counted_as_cores() {
        let mut i = base();
        i.core_masks.clear();
        i.affinity_mask = 0xF;
        let m = Machine::from_inputs(&i);
        assert_eq!((m.logical_processors, m.physical_cores), (4, 4));
    }

    #[test]
    fn an_empty_mask_still_gives_one_thread() {
        let m = Machine::from_inputs(&Inputs::default());
        assert_eq!((m.logical_processors, m.physical_cores), (1, 1));
        assert_eq!((m.batch_threads(), m.live_threads()), (1, 1));
    }

    fn record(class: u8, flags: u8, groups: &[(u64, u16)]) -> Vec<u8> {
        let size = 32 + 16 * groups.len();
        let mut r = vec![0u8; size];
        r[0..4].copy_from_slice(&0u32.to_le_bytes());
        r[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        r[8] = flags;
        r[9] = class;
        r[30..32].copy_from_slice(&(groups.len() as u16).to_le_bytes());
        for (g, (mask, group)) in groups.iter().enumerate() {
            let b = 32 + g * 16;
            r[b..b + 8].copy_from_slice(&mask.to_le_bytes());
            r[b + 8..b + 10].copy_from_slice(&group.to_le_bytes());
        }
        r
    }

    #[test]
    fn core_records_parse_to_one_mask_per_core() {
        let mut bytes = record(1, 1, &[(0b11, 0)]);
        bytes.extend(record(0, 0, &[(1 << 16, 0)]));
        bytes.extend(record(0, 0, &[(1 << 17, 0)]));
        assert_eq!(core_masks(&bytes), vec![0b11, 1 << 16, 1 << 17]);
    }

    #[test]
    fn records_in_other_processor_groups_are_skipped_and_garbage_stops_the_parse() {
        let mut bytes = record(0, 0, &[(0b1, 1)]);
        bytes.extend(record(0, 0, &[(0b100, 0)]));
        assert_eq!(core_masks(&bytes), vec![0b100]);
        // A record of size zero ends the walk instead of looping.
        let mut bad = record(0, 0, &[(0b1, 0)]);
        bad.extend([0u8; 16]);
        assert_eq!(core_masks(&bad), vec![0b1]);
        assert!(core_masks(&[1, 2, 3]).is_empty());
    }

    #[test]
    fn probe_is_consistent_with_the_process() {
        let m = probe();
        assert!(m.logical_processors >= 1);
        assert!(m.physical_cores >= 1 && m.physical_cores <= m.logical_processors);
        #[cfg(windows)]
        {
            assert!(m.host_total_ram_bytes > 0);
            assert!(m.available_ram_bytes > 0);
        }
    }
}
