//! The machine fingerprint stored in `capability.json`. A changed fingerprint (hardware, drivers,
//! app version or model manifest) shows a banner and re-runs the quick check.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuPrint {
    pub name: String,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub os: String,
    pub cpu_brand: String,
    pub logical_processors: u32,
    pub physical_cores: u32,
    /// Total RAM in whole GiB, so a few MB of difference does not change the fingerprint.
    pub total_ram_gib: u32,
    pub gpus: Vec<GpuPrint>,
    pub app_version: String,
    /// SHA-256 of `models/manifest.json` (model files, hashes, rankings).
    pub manifest_hash: String,
    pub emulated: Option<String>,
}

impl Fingerprint {
    /// Short id for the status bar, for example `3F2A-91C7`.
    pub fn id(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        format!(
            "{:02X}{:02X}-{:02X}{:02X}",
            digest[0], digest[1], digest[2], digest[3]
        )
    }

    /// What changed against an earlier fingerprint, in words; empty when they are equal.
    pub fn differences(&self, earlier: &Fingerprint) -> Vec<String> {
        let mut out = Vec::new();
        let mut check = |what: &str, a: String, b: String| {
            if a != b {
                out.push(format!("{what}: {b} -> {a}"));
            }
        };
        check("operating system", self.os.clone(), earlier.os.clone());
        check(
            "processor",
            self.cpu_brand.clone(),
            earlier.cpu_brand.clone(),
        );
        check(
            "logical processors",
            self.logical_processors.to_string(),
            earlier.logical_processors.to_string(),
        );
        check(
            "physical cores",
            self.physical_cores.to_string(),
            earlier.physical_cores.to_string(),
        );
        check(
            "memory (GiB)",
            self.total_ram_gib.to_string(),
            earlier.total_ram_gib.to_string(),
        );
        check(
            "graphics adapters and drivers",
            format!("{:?}", self.gpus),
            format!("{:?}", earlier.gpus),
        );
        check(
            "app version",
            self.app_version.clone(),
            earlier.app_version.clone(),
        );
        check(
            "model manifest",
            self.manifest_hash.clone(),
            earlier.manifest_hash.clone(),
        );
        check(
            "emulation profile",
            format!("{:?}", self.emulated),
            format!("{:?}", earlier.emulated),
        );
        out
    }
}
