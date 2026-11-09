//! Parsing of RelationProcessorCore records and the core choice per profile (safe code).

use anyhow::{Result, anyhow, bail};

/// One physical core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core {
    /// Higher means faster (performance cores); the lowest class is the efficiency cores.
    pub efficiency_class: u8,
    pub smt: bool,
    /// Logical processor numbers (group 0) of this core.
    pub logical: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// 4 efficiency cores, 4 threads.
    EmuLow,
    /// 2 performance cores with SMT plus 8 efficiency cores, 12 threads.
    EmuMid,
    /// The whole machine.
    Native,
}

impl std::str::FromStr for Profile {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "emu-low" => Ok(Self::EmuLow),
            "emu-mid" => Ok(Self::EmuMid),
            "native" => Ok(Self::Native),
            other => Err(anyhow!(
                "unknown profile {other}; use emu-low, emu-mid or native"
            )),
        }
    }
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::EmuLow => "emu-low",
            Self::EmuMid => "emu-mid",
            Self::Native => "native",
        }
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(o)?, *b.get(o + 1)?]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(o)?,
        *b.get(o + 1)?,
        *b.get(o + 2)?,
        *b.get(o + 3)?,
    ]))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    let mut a = [0u8; 8];
    a.copy_from_slice(b.get(o..o + 8)?);
    Some(u64::from_le_bytes(a))
}

/// Parses SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX records (x64 layout):
/// Relationship u32 @0, Size u32 @4, PROCESSOR_RELATIONSHIP @8 { Flags u8 @8,
/// EfficiencyClass u8 @9, Reserved [u8; 20], GroupCount u16 @30, GroupMask @32: [GROUP_AFFINITY
/// { Mask u64, Group u16, Reserved [u16; 3] }] (16 bytes each) }.
pub fn cores(bytes: &[u8]) -> Result<Vec<Core>> {
    const RELATION_PROCESSOR_CORE: u32 = 0;
    const LTP_PC_SMT: u8 = 1;
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let rel = u32_at(bytes, off).ok_or_else(|| anyhow!("truncated record"))?;
        let size = u32_at(bytes, off + 4).ok_or_else(|| anyhow!("truncated record"))? as usize;
        if size == 0 {
            bail!("zero-size processor record");
        }
        if rel == RELATION_PROCESSOR_CORE {
            let flags = *bytes
                .get(off + 8)
                .ok_or_else(|| anyhow!("truncated core"))?;
            let class = *bytes
                .get(off + 9)
                .ok_or_else(|| anyhow!("truncated core"))?;
            let groups = u16_at(bytes, off + 30).ok_or_else(|| anyhow!("truncated core"))?;
            let mut logical = Vec::new();
            for g in 0..usize::from(groups) {
                let base = off + 32 + g * 16;
                let mask = u64_at(bytes, base).ok_or_else(|| anyhow!("truncated group mask"))?;
                let group =
                    u16_at(bytes, base + 8).ok_or_else(|| anyhow!("truncated group mask"))?;
                if group != 0 {
                    bail!(
                        "processor group {group}: machines with more than 64 logical processors are not supported"
                    );
                }
                logical.extend((0..64u32).filter(|b| mask & (1u64 << b) != 0));
            }
            out.push(Core {
                efficiency_class: class,
                smt: flags & LTP_PC_SMT != 0,
                logical,
            });
        }
        off += size;
    }
    if out.is_empty() {
        bail!("no processor cores reported");
    }
    Ok(out)
}

/// The cores a profile uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub mask: usize,
    pub logical: Vec<u32>,
    pub classes: Vec<u8>,
}

/// Picks cores for `profile`: `None` for native (no affinity limit).
pub fn select(cores: &[Core], profile: Profile) -> Result<Option<Selection>> {
    let min = cores.iter().map(|c| c.efficiency_class).min().unwrap_or(0);
    let max = cores.iter().map(|c| c.efficiency_class).max().unwrap_or(0);
    if profile != Profile::Native && min == max {
        bail!(
            "this CPU has one efficiency class; emu profiles need a hybrid CPU (performance and efficiency cores)"
        );
    }
    let e_cores: Vec<&Core> = cores.iter().filter(|c| c.efficiency_class == min).collect();
    let p_cores: Vec<&Core> = cores.iter().filter(|c| c.efficiency_class == max).collect();
    let chosen: Vec<&Core> = match profile {
        Profile::Native => return Ok(None),
        Profile::EmuLow => {
            if e_cores.len() < 4 {
                bail!("emu-low needs 4 efficiency cores, found {}", e_cores.len());
            }
            e_cores.into_iter().take(4).collect()
        }
        Profile::EmuMid => {
            if e_cores.len() < 8 || p_cores.len() < 2 {
                bail!("emu-mid needs 2 performance and 8 efficiency cores");
            }
            p_cores
                .into_iter()
                .take(2)
                .chain(e_cores.into_iter().take(8))
                .collect()
        }
    };
    let mut logical = Vec::new();
    let mut classes = Vec::new();
    let mut mask = 0usize;
    for c in chosen {
        for &l in &c.logical {
            mask |= 1usize << l;
            logical.push(l);
            classes.push(c.efficiency_class);
        }
    }
    Ok(Some(Selection {
        mask,
        logical,
        classes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(class: u8, smt: bool, mask: u64) -> Vec<u8> {
        let mut r = vec![0u8; 48];
        r[0..4].copy_from_slice(&0u32.to_le_bytes());
        r[4..8].copy_from_slice(&48u32.to_le_bytes());
        r[8] = u8::from(smt);
        r[9] = class;
        r[30..32].copy_from_slice(&1u16.to_le_bytes());
        r[32..40].copy_from_slice(&mask.to_le_bytes());
        r
    }

    /// A 14900HX-like layout: 8 P-cores with SMT (logical 0..15), 16 E-cores (16..31).
    fn legion() -> Vec<u8> {
        let mut b = Vec::new();
        for i in 0..8u64 {
            b.extend(record(1, true, 0b11 << (2 * i)));
        }
        for i in 16..32u64 {
            b.extend(record(0, false, 1 << i));
        }
        b
    }

    #[test]
    fn parses_and_selects() -> Result<()> {
        let cores = cores(&legion())?;
        assert_eq!(cores.len(), 24);
        assert_eq!(cores[0].logical, vec![0, 1]);
        assert!(cores[0].smt);

        let low = select(&cores, Profile::EmuLow)?.ok_or_else(|| anyhow!("none"))?;
        assert_eq!(low.logical, vec![16, 17, 18, 19]);
        assert!(low.classes.iter().all(|&c| c == 0));
        assert_eq!(low.mask.count_ones(), 4);

        let mid = select(&cores, Profile::EmuMid)?.ok_or_else(|| anyhow!("none"))?;
        assert_eq!(mid.mask.count_ones(), 12);
        assert_eq!(&mid.logical[..4], &[0, 1, 2, 3]);
        assert_eq!(mid.classes.iter().filter(|&&c| c == 1).count(), 4);

        assert!(select(&cores, Profile::Native)?.is_none());
        Ok(())
    }

    #[test]
    fn non_hybrid_cpu_is_refused() -> Result<()> {
        let mut b = Vec::new();
        for i in 0..8u64 {
            b.extend(record(0, true, 0b11 << (2 * i)));
        }
        let cores = cores(&b)?;
        assert!(select(&cores, Profile::EmuLow).is_err());
        assert!(select(&cores, Profile::Native)?.is_none());
        Ok(())
    }
}
