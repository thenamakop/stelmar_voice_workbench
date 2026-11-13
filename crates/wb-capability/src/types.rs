//! The data of `capability.json` (schema version 1): what the system check measured and what the
//! rules decided from it. Pure data, no I/O. Sizes are bytes, speeds are real-time factors (RTF =
//! processing time / audio time).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::fingerprint::Fingerprint;

pub const SCHEMA_VERSION: u32 = 1;
pub const GIB: u64 = 1 << 30;

/// The languages of a session. Spoken Hindi and Urdu are one language whose output script the
/// organiser picks (GATES, 13 Nov 2026), so a session carries `En` plus either `Hi` (Devanagari)
/// or `Ur` (Nastaliq); the profile measures all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Hi,
    Ur,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Hi, Lang::Ur];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Hi => "hi",
            Lang::Ur => "ur",
        }
    }

    pub fn parse(s: &str) -> Option<Lang> {
        match s {
            "en" => Some(Lang::En),
            "hi" => Some(Lang::Hi),
            "ur" => Some(Lang::Ur),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Hi => "Hindi (Devanagari)",
            Lang::Ur => "Urdu (Nastaliq)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Beam 5 with temperature fallback on 15 s chunks (the rolling and night passes).
    Rolling,
    /// Greedy on utterance-length chunks (near-live text, and the tier G live stand-in).
    NearLive,
}

/// Which devices decode. `Gpu.index` is the index in the vulkan engine's de-duplicated device list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceConfig {
    Cpu,
    Gpu {
        index: u32,
    },
    /// A CPU engine and a GPU engine on separate work units.
    Hybrid {
        gpu: u32,
    },
}

impl DeviceConfig {
    pub fn gpu_index(self) -> Option<u32> {
        match self {
            DeviceConfig::Cpu => None,
            DeviceConfig::Gpu { index } => Some(index),
            DeviceConfig::Hybrid { gpu } => Some(gpu),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuKind {
    Integrated,
    Discrete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gpu {
    /// Index in the vulkan engine's device list (what `Device::Gpu { index }` means).
    pub index: u32,
    pub name: String,
    pub vendor: String,
    pub kind: GpuKind,
    pub dedicated_bytes: u64,
    /// DXGI "local" memory. On an integrated GPU this is system RAM and is counted as RAM.
    pub local_bytes: u64,
    pub shared_bytes: u64,
    /// True when the shared figure could not be read and half of system RAM was assumed.
    pub shared_assumed: bool,
    pub driver: Option<String>,
    /// The vulkan engine lists the device and ran on it.
    pub vulkan_usable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    pub role: String,
    pub path: String,
    pub free_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hardware {
    pub os: String,
    pub cpu_brand: String,
    pub logical_processors: u32,
    pub physical_cores: u32,
    pub cpu_features: Vec<String>,
    pub avx2: bool,
    pub avx512: bool,
    pub total_ram_bytes: u64,
    pub available_ram_bytes: u64,
    pub gpus: Vec<Gpu>,
    pub disks: Vec<Disk>,
    /// `None` when Windows could not say.
    pub on_mains_power: Option<bool>,
    pub audio_inputs: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub variant: String,
    pub usable: bool,
    pub reason: Option<String>,
    pub info_ms: Option<u64>,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub model: String,
    pub lang: Lang,
    pub mode: Mode,
    pub config: DeviceConfig,
    /// The engine build that ran it: "avx2", "avx512", "vulkan".
    pub engine_build: String,
    pub device_name: String,
    pub threads: u32,
    /// How it was measured: clip, excerpt length, runs, settings.
    pub method: String,
    pub clip_seconds: f64,
    pub rtf: f64,
    pub warmup_rtf: Option<f64>,
    pub load_s: f64,
    pub peak_working_set_bytes: u64,
    pub peak_commit_bytes: u64,
    pub gpu_local_bytes: Option<u64>,
    pub truncated_chunks: u32,
    /// The measurement is the tier G live stand-in (near-live settings on the GPU), not a streaming run.
    #[serde(default)]
    pub live_standin: bool,
}

/// A CPU engine and a GPU engine decoding at the same time, on different clips of the same model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridMeasurement {
    pub model: String,
    pub lang: Lang,
    pub gpu: u32,
    pub cpu_threads: u32,
    pub gpu_rtf_concurrent: f64,
    pub cpu_rtf_concurrent: f64,
    /// Peak commit of the CPU engine and of the GPU engine while they ran together (0 = not recorded: the
    /// rules then use the peaks of the single-device cells of the same model).
    #[serde(default)]
    pub cpu_peak_commit_bytes: u64,
    #[serde(default)]
    pub gpu_peak_commit_bytes: u64,
    /// The graphics adapter's memory in use during the test.
    #[serde(default)]
    pub gpu_local_bytes: Option<u64>,
    pub method: String,
}

impl HybridMeasurement {
    /// Seconds of audio decoded per second by both engines together.
    pub fn combined_throughput(&self) -> f64 {
        1.0 / self.gpu_rtf_concurrent + 1.0 / self.cpu_rtf_concurrent
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecorderSafety {
    pub config: DeviceConfig,
    pub duration_s: f64,
    pub overruns: u32,
    pub driver_xruns: u32,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Quick,
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckInfo {
    pub kind: CheckKind,
    /// Some cells were left for later (the quick check ran long): the verdict is provisional.
    pub partial: bool,
    pub started_at: String,
    pub duration_s: f64,
    /// The time the quick check must finish in on the low-baseline laptop.
    pub bar_s: f64,
    pub excerpt_seconds: f64,
}

/// Everything the check measured and the machine looks like; the input of `decide`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub schema_version: u32,
    pub app_version: String,
    pub created_at: String,
    /// `Some("emu-low")` etc. under wb-emulate: speed bars get the 25% headroom and the profile is
    /// never valid on another machine.
    pub emulated: Option<String>,
    pub fingerprint: Fingerprint,
    pub check: CheckInfo,
    pub hardware: Hardware,
    pub engines: Vec<EngineStatus>,
    pub measurements: Vec<Measurement>,
    pub hybrid: Option<HybridMeasurement>,
    pub recorder_safety: Vec<RecorderSafety>,
    /// Window-hook programs found running (RTSS, MSI Afterburner, ...).
    pub injectors: Vec<String>,
    /// Steps the check left out, and why (nothing to measure, a prediction, left for the full check).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<Skipped>,
}

/// A step the check did not run. Shown in the evidence; never an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub step: String,
    pub reason: String,
}

/// One ranked candidate of a language and role (from `provisional_rank` in `models/manifest.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ranked {
    pub model: String,
    pub bytes: u64,
    /// No local, independent WER backs this model's place (Phase 5 decides).
    pub quality_unverified: bool,
    /// The model is in `validated_roles` for this role. Near-live text is offered only for these.
    pub validated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Rankings {
    pub rolling: BTreeMap<Lang, Vec<Ranked>>,
    pub night: BTreeMap<Lang, Vec<Ranked>>,
    pub near_live: BTreeMap<Lang, Vec<Ranked>>,
    /// Candidates for the tier G live stand-in.
    pub live: BTreeMap<Lang, Vec<Ranked>>,
    /// Every place in the table is provisional until Phase 5.
    pub provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Options {
    /// Hours between the last session and 08:00 the next day.
    pub night_window_hours: f64,
    /// Hours of audio a room records in a day (PLAN section 4: about 3).
    pub day_audio_hours: f64,
    /// Free RAM that must remain after a model's measured peak.
    pub min_free_ram_bytes: u64,
    /// A hybrid configuration is chosen only if it measured at least this much faster (a ratio: 1.15 is 15%;
    /// owner's decision of 14 Nov 2026, after the gain on emu-mid measured +11.7% in one run and +3.4% in the next).
    pub hybrid_min_gain: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            night_window_hours: 12.0,
            day_audio_hours: 3.0,
            min_free_ram_bytes: 3 * GIB,
            hybrid_min_gain: 1.15,
        }
    }
}

// ----------------------------------------------------------------------------------- decisions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    #[serde(rename = "G")]
    G,
    #[serde(rename = "C+")]
    CPlus,
    #[serde(rename = "C")]
    C,
    #[serde(rename = "R")]
    R,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::G => "G",
            Tier::CPlus => "C+",
            Tier::C => "C",
            Tier::R => "R",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bars {
    pub rolling_rtf: f64,
    pub near_live_rtf: f64,
    pub live_standin_rtf: f64,
    /// 0.75 under emulation, otherwise 1.0.
    pub headroom_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigChoice {
    pub config: DeviceConfig,
    /// "CPU + Intel(R) UHD Graphics (hybrid)" etc., for the dialog.
    pub label: String,
    pub why: String,
    pub cpu_threads: u32,
    pub recorder_safety_passed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollingPick {
    pub model: String,
    pub rtf: f64,
    pub meets_bar: bool,
    pub quality_unverified: bool,
    /// Decodes of a chunk that still fit the bar: floor(bar / rtf).
    pub k_max: u32,
    /// Minutes after Stop at which a 90-minute talk's transcript is complete (PLAN section 4).
    pub ready_after_stop_min: f64,
    /// RTF over 1: processing will lag behind recording.
    pub lags: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NightPick {
    pub model: String,
    pub rtf: f64,
    pub hours_for_a_day: f64,
    pub quality_unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveDecision {
    pub offered: bool,
    /// "streaming" (tier G) or "near_live" (tier C+) when offered.
    pub kind: Option<String>,
    pub model: Option<String>,
    pub rtf: Option<f64>,
    /// Why not, in plain words (a switch beyond the verdict can lift these).
    pub soft_reasons: Vec<String>,
    /// Why not, and the operator cannot override it (cannot keep up with real time, or lost speech).
    pub hard_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LangDecision {
    pub rolling: Option<RollingPick>,
    pub night: Option<NightPick>,
    pub night_note: Option<String>,
    pub live: LiveDecision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hinglish {
    pub ok: bool,
    pub rtf: Option<f64>,
    /// "measured" (a hinglish cell), "proxy: large-v3-q5_0" (same size) or "unmeasured".
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub tier: Tier,
    pub tier_reasons: Vec<String>,
    /// Set when the tier is R.
    pub record_only_reason: Option<String>,
    pub bars: Bars,
    pub config: Option<ConfigChoice>,
    pub languages: BTreeMap<Lang, LangDecision>,
    /// Rule (d): the Hinglish model for every chunk fits the bar.
    pub hinglish: Hinglish,
    /// Rule (c): decoding a chunk both ways in an en+hi session fits the bar.
    pub dual_decode_en_hi: bool,
    pub provisional: bool,
    pub partial: bool,
    /// Plain-words verdict for the System screen.
    pub verdict: Vec<String>,
    pub process_while_recording: bool,
    pub pause_on_battery: bool,
}

// ------------------------------------------------------------------------------------ overrides

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideKind {
    ProcessWhileRecording,
    PauseOnBattery,
    AllowLiveAnyway,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverrideRecord {
    pub kind: OverrideKind,
    pub enabled: bool,
    pub at: String,
    /// The operator confirmed the Dialog that explains the risk.
    pub confirmed: bool,
}

/// `capability.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capability {
    #[serde(flatten)]
    pub profile: Profile,
    pub decision: Decision,
    pub overrides: Vec<OverrideRecord>,
}
