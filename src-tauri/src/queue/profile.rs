//! What the queue reads from the capability profile in use (`syscheck::store::in_use`, never the file directly:
//! after a fingerprint change that function hands back safe defaults, which refuse processing while recording
//! and live text, and the queue honours them).

use std::collections::BTreeMap;

use wb_capability::{Capability, DeviceConfig, OverrideKind, Tier};

/// One measured cell the start gate and the memory limit use: the peak commit of an engine running a model on
/// a kind of device, and the graphics memory it held.
#[derive(Debug, Clone, PartialEq)]
struct Peak {
    model: String,
    gpu: bool,
    peak_commit_bytes: u64,
    gpu_local_bytes: u64,
    engine_build: String,
    rtf: f64,
}

#[derive(Debug, Clone)]
pub struct ProfileView {
    /// Tier R: this computer only records.
    pub record_only: bool,
    pub record_only_reason: Option<String>,
    /// Processing while recording is allowed (the recorder safety test passed for the chosen setup and the
    /// operator has not switched it off). False also after a fingerprint change.
    pub process_while_recording: bool,
    pub pause_on_battery: bool,
    pub config: Option<DeviceConfig>,
    /// The graphics adapter index of a graphics or hybrid configuration.
    pub gpu_index: Option<u32>,
    pub cpu_threads: u32,
    /// What differs between this computer and the profile (safe defaults are in force when not empty).
    pub changes: Vec<String>,
    rolling: BTreeMap<String, String>,
    night: BTreeMap<String, String>,
    peaks: Vec<Peak>,
}

/// The state of an override as the System screen shows it: the last logged change, else the verdict's default.
/// A later check whose recorder safety test did not pass switches processing while recording off again.
pub fn override_state(cap: &Capability, kind: OverrideKind) -> bool {
    let last = cap.overrides.iter().rev().find(|o| o.kind == kind);
    if let Some(o) = last {
        if kind == OverrideKind::ProcessWhileRecording && !cap.decision.process_while_recording {
            return false;
        }
        return o.enabled && o.confirmed;
    }
    match kind {
        OverrideKind::ProcessWhileRecording => cap.decision.process_while_recording,
        OverrideKind::PauseOnBattery => cap.decision.pause_on_battery,
        OverrideKind::AllowLiveAnyway => false,
    }
}

impl ProfileView {
    pub fn from_capability(cap: &Capability, changes: Vec<String>) -> Self {
        let d = &cap.decision;
        let mut rolling = BTreeMap::new();
        let mut night = BTreeMap::new();
        for (lang, ld) in &d.languages {
            if let Some(r) = &ld.rolling {
                rolling.insert(lang.code().to_owned(), r.model.clone());
            }
            if let Some(n) = &ld.night {
                night.insert(lang.code().to_owned(), n.model.clone());
            }
        }
        let peaks = cap
            .profile
            .measurements
            .iter()
            .map(|m| Peak {
                model: m.model.clone(),
                gpu: !matches!(m.config, DeviceConfig::Cpu),
                peak_commit_bytes: m.peak_commit_bytes,
                gpu_local_bytes: m.gpu_local_bytes.unwrap_or(0),
                engine_build: m.engine_build.clone(),
                rtf: m.rtf,
            })
            .collect();
        Self {
            record_only: d.tier == Tier::R,
            record_only_reason: d.record_only_reason.clone(),
            process_while_recording: override_state(cap, OverrideKind::ProcessWhileRecording)
                && d.process_while_recording,
            pause_on_battery: override_state(cap, OverrideKind::PauseOnBattery),
            config: d.config.as_ref().map(|c| c.config),
            gpu_index: d.config.as_ref().and_then(|c| c.config.gpu_index()),
            cpu_threads: d.config.as_ref().map_or(1, |c| c.cpu_threads.max(1)),
            changes,
            rolling,
            night,
            peaks,
        }
    }

    /// `models_by_lang` ids of the rolling pass for the session's languages: each language's own pick, and `*`
    /// (any other language) from the first language that has one. Empty when no language has a pick.
    pub fn rolling_models(&self, languages: &[String]) -> BTreeMap<String, String> {
        Self::pick(&self.rolling, languages)
    }

    /// The night pass models: only languages that have a night pick; the others keep their rolling model.
    pub fn night_models(&self, languages: &[String]) -> BTreeMap<String, String> {
        let mut m = Self::pick(&self.rolling, languages);
        for l in languages {
            if let Some(n) = self.night.get(l) {
                m.insert(l.clone(), n.clone());
            }
        }
        if let Some(first) = languages.iter().find_map(|l| self.night.get(l).cloned()) {
            m.insert("*".into(), first);
        }
        m
    }

    /// Whether the chosen configuration decodes on a graphics adapter (alone or beside the processor).
    pub fn uses_gpu(&self) -> bool {
        matches!(
            self.config,
            Some(DeviceConfig::Gpu { .. } | DeviceConfig::Hybrid { .. })
        )
    }

    /// The real-time factor of each language's night model in the chosen configuration (only languages that
    /// have a night model). The night plan counts only their audio.
    pub fn night_rtf(&self, languages: &[String]) -> BTreeMap<String, f64> {
        let gpu = self.uses_gpu();
        languages
            .iter()
            .filter_map(|l| {
                let model = self.night.get(l)?;
                let rtf = self.rtf_for(model, gpu);
                // A model that was never timed counts as slow (1.0), not free.
                Some((l.clone(), if rtf > 0.0 { rtf } else { 1.0 }))
            })
            .collect()
    }

    fn pick(from: &BTreeMap<String, String>, languages: &[String]) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for l in languages {
            if let Some(model) = from.get(l) {
                m.insert(l.clone(), model.clone());
            }
        }
        if let Some(any) = languages.iter().find_map(|l| from.get(l).cloned()) {
            m.insert("*".into(), any);
        }
        m
    }

    /// The measured peak commit and graphics memory of `model` on a processor or graphics engine. A model with
    /// no measurement uses the largest peak the profile holds (AGENTS.md: never assume a small one).
    pub fn peak_for(&self, model: &str, gpu: bool) -> (u64, u64) {
        let exact = self
            .peaks
            .iter()
            .filter(|p| p.model == model && p.gpu == gpu)
            .max_by_key(|p| p.peak_commit_bytes);
        if let Some(p) = exact {
            return (p.peak_commit_bytes, p.gpu_local_bytes);
        }
        let same_model = self
            .peaks
            .iter()
            .filter(|p| p.model == model)
            .max_by_key(|p| p.peak_commit_bytes);
        if let Some(p) = same_model {
            return (p.peak_commit_bytes, if gpu { p.gpu_local_bytes } else { 0 });
        }
        self.largest_peak(gpu)
    }

    fn largest_peak(&self, gpu: bool) -> (u64, u64) {
        self.peaks
            .iter()
            .max_by_key(|p| p.peak_commit_bytes)
            .map_or((0, 0), |p| {
                (p.peak_commit_bytes, if gpu { p.gpu_local_bytes } else { 0 })
            })
    }

    /// The slowest measured real-time factor of `model` on a kind of device (processing seconds per audio second),
    /// or of any model there when `model` has none. 0 when nothing was measured.
    pub fn rtf_for(&self, model: &str, gpu: bool) -> f64 {
        let of = |only_model: bool| {
            self.peaks
                .iter()
                .filter(|p| p.gpu == gpu && (!only_model || p.model == model))
                .map(|p| p.rtf)
                .fold(0.0, f64::max)
        };
        let exact = of(true);
        if exact > 0.0 { exact } else { of(false) }
    }

    /// The engine build the profile measured for a kind of device (`avx2`, `avx512`, `vulkan`), if it did.
    pub fn engine_build(&self, gpu: bool) -> Option<&str> {
        self.peaks
            .iter()
            .find(|p| p.gpu == gpu)
            .map(|p| p.engine_build.as_str())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wb_capability::OverrideRecord;

    fn tier_g() -> Capability {
        wb_capability::parse_capability(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ))
        .unwrap()
    }

    fn tier_r() -> Capability {
        wb_capability::parse_capability(include_str!(
            "../../../crates/wb-capability/fixtures/tier-r.json"
        ))
        .unwrap()
    }

    #[test]
    fn tier_r_only_records_and_processes_nothing() {
        let v = ProfileView::from_capability(&tier_r(), vec![]);
        assert!(v.record_only);
        assert!(!v.process_while_recording);
        assert!(v.config.is_none());
    }

    #[test]
    fn processing_while_recording_follows_the_profile_and_the_operator() {
        let mut cap = tier_g();
        let v = ProfileView::from_capability(&cap, vec![]);
        assert_eq!(
            v.process_while_recording,
            cap.decision.process_while_recording
        );
        // The operator switched it off (confirmed): it is off.
        cap.overrides.push(OverrideRecord {
            kind: OverrideKind::ProcessWhileRecording,
            enabled: false,
            at: "t".into(),
            confirmed: true,
        });
        assert!(!ProfileView::from_capability(&cap, vec![]).process_while_recording);
        // A profile whose safety test did not pass never processes while recording, whatever an old override said.
        let mut weak = tier_g();
        weak.decision.process_while_recording = false;
        weak.overrides.push(OverrideRecord {
            kind: OverrideKind::ProcessWhileRecording,
            enabled: true,
            at: "t".into(),
            confirmed: true,
        });
        assert!(!ProfileView::from_capability(&weak, vec![]).process_while_recording);
    }

    #[test]
    fn safe_defaults_after_a_fingerprint_change_refuse_processing_while_recording() {
        let cap = tier_g();
        let safe = wb_capability::inuse::safe_defaults(&cap, "the computer changed");
        let v = ProfileView::from_capability(&safe, vec!["the computer changed".into()]);
        assert!(!v.process_while_recording);
        assert!(!v.record_only, "post-processing stays as measured");
        assert_eq!(v.changes.len(), 1);
    }

    #[test]
    fn models_follow_the_sessions_languages_and_an_unmeasured_model_takes_the_largest_peak() {
        let cap = tier_g();
        let v = ProfileView::from_capability(&cap, vec![]);
        let m = v.rolling_models(&["en".into(), "hi".into()]);
        assert!(m.contains_key("*"), "{m:?}");
        assert!(m.len() >= 2);
        let (peak, _) = v.peak_for("a-model-nobody-measured", false);
        let largest = cap
            .profile
            .measurements
            .iter()
            .map(|m| m.peak_commit_bytes)
            .max()
            .unwrap_or(0);
        assert_eq!(peak, largest);
    }

    #[test]
    fn a_session_without_a_night_model_gets_no_night_pass() {
        let v = ProfileView::from_capability(&tier_g(), vec![]);
        assert!(v.night_rtf(&["xx".into()]).is_empty());
    }
}
