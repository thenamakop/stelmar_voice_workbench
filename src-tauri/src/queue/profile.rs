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
    /// The language of the clip the cell was measured on (`en`, `hi`, `ur`).
    lang: String,
    /// A rolling-settings cell (the near-live stand-in cells are not what the queue runs).
    rolling: bool,
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
    /// The Hinglish model measured on this computer that meets the rolling bar (`Hinglish::chosen_model`): a session in
    /// English and Hindi decodes every chunk with it (rule (d2), Phase 5, 25 Nov 2026). None where it was not measured
    /// or does not fit the bar or the memory: today's per-language choice stays.
    hinglish: Option<String>,
    /// The measured Hinglish model that does not keep up while recording but fits the night window and the memory
    /// (`Hinglish::night_model`): the night pass may decode English + Hindi sessions with it (Phase 5, 26 Nov 2026);
    /// the queue plans the real night from the audio and uses it only when that finishes before 08:00.
    hinglish_night: Option<String>,
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
                lang: m.lang.code().to_owned(),
                rolling: m.mode == wb_capability::Mode::Rolling,
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
            hinglish: d.hinglish.chosen_model().map(str::to_owned),
            hinglish_night: d.hinglish.night_model().map(str::to_owned),
            peaks,
        }
    }

    /// A measured cell for the tests: `model` on a clip of `lang`, on the chosen kind of device.
    #[cfg(test)]
    pub fn set_cell_for_test(&mut self, model: &str, lang: &str, rtf: f64) {
        let gpu = self.uses_gpu();
        self.peaks.push(Peak {
            model: model.to_owned(),
            lang: lang.to_owned(),
            rolling: true,
            gpu,
            peak_commit_bytes: 0,
            gpu_local_bytes: 0,
            engine_build: String::new(),
            rtf,
        });
    }

    /// Whether the night pass may decode a session with these languages with the Hinglish model: English and Hindi
    /// together, a measured model that fits the night, and the recording was not already decoded with it.
    pub fn may_use_hinglish_at_night(&self, languages: &[String]) -> bool {
        self.hinglish_night.is_some()
            && !self.uses_hinglish(languages)
            && languages.iter().any(|l| l == "en")
            && languages.iter().any(|l| l == "hi")
    }

    /// Whether a session in English and Hindi is left with today's models, because the Hinglish model was not
    /// measured here, is too slow, or does not fit (the session is "best finished on a faster machine" unless the
    /// night pass turns out to run the Hinglish model).
    pub fn lacks_hinglish(&self, languages: &[String]) -> bool {
        !self.uses_hinglish(languages)
            && languages.iter().any(|l| l == "en")
            && languages.iter().any(|l| l == "hi")
    }

    /// The name of the Hinglish model the night pass may use (for the log and the marker).
    pub fn hinglish_night_model(&self) -> Option<&str> {
        self.hinglish_night.as_deref()
    }

    /// The models of a night job that decodes with the Hinglish model: every language and the catch-all.
    pub fn hinglish_night_models(&self, languages: &[String]) -> BTreeMap<String, String> {
        let Some(model) = self.hinglish_night.as_ref() else {
            return BTreeMap::new();
        };
        let mut m: BTreeMap<String, String> = languages
            .iter()
            .map(|l| (l.clone(), model.clone()))
            .collect();
        m.insert("*".into(), model.clone());
        m
    }

    /// The speed of the Hinglish night model for each language of a session (the slowest measured one where a
    /// language has no cell of its own, never 0): the night plan's `night_rtf` and `decode_rtf` for such sessions.
    pub fn hinglish_night_rtf(&self, languages: &[String]) -> BTreeMap<String, f64> {
        let Some(model) = self.hinglish_night.as_ref() else {
            return BTreeMap::new();
        };
        let gpu = self.uses_gpu();
        languages
            .iter()
            .map(|l| {
                let rtf = self.rtf_for_lang(model, l, gpu);
                (l.clone(), if rtf > 0.0 { rtf } else { 1.0 })
            })
            .collect()
    }

    /// Whether a session with these languages decodes every chunk with the Hinglish model: English and Hindi
    /// together, and the system check chose the model. Urdu sessions and single-language sessions keep their picks.
    pub fn uses_hinglish(&self, languages: &[String]) -> bool {
        self.hinglish.is_some()
            && languages.iter().any(|l| l == "en")
            && languages.iter().any(|l| l == "hi")
    }

    /// `models_by_lang` ids of the rolling pass for the session's languages: each language's own pick, and `*`
    /// (any other language) from the first language that has one. Empty when no language has a pick.
    pub fn rolling_models(&self, languages: &[String]) -> BTreeMap<String, String> {
        if let Some(model) = self
            .hinglish
            .as_ref()
            .filter(|_| self.uses_hinglish(languages))
        {
            // One model for every chunk of the session: detection and decoding by the same model, so a Hindi
            // sentence is never decoded as English text by a model that translates it.
            let mut m: BTreeMap<String, String> = languages
                .iter()
                .map(|l| (l.clone(), model.clone()))
                .collect();
            m.insert("*".into(), model.clone());
            return m;
        }
        Self::pick(&self.rolling, languages)
    }

    /// The night pass models: only languages that have a night pick; the others keep their rolling model.
    pub fn night_models(&self, languages: &[String]) -> BTreeMap<String, String> {
        if self.uses_hinglish(languages) {
            // The rolling pass already used the Hinglish model: no night model is more accurate for these sessions.
            return self.rolling_models(languages);
        }
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

    /// The class of device the night allowance is chosen by: a graphics adapter alone, or the processor (a hybrid
    /// configuration, a processor-only one, and a profile without a configuration all count as the processor: the
    /// estimate must not be more hopeful than the slower engine of the pair).
    pub fn night_class(&self) -> wb_queue::speed::DeviceClass {
        match self.config {
            Some(DeviceConfig::Gpu { .. }) => wb_queue::speed::DeviceClass::Gpu,
            _ => wb_queue::speed::DeviceClass::Cpu,
        }
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

    /// The real-time factor of every language the night job decodes, each at the model the job uses for it: the
    /// night model, the rolling model for a language without one, or the job's catch-all model. The job decodes the
    /// whole session, so the night plan counts all of it at these speeds (23 Nov 2026).
    pub fn night_decode_rtf(&self, languages: &[String]) -> BTreeMap<String, f64> {
        let gpu = self.uses_gpu();
        let models = self.night_models(languages);
        languages
            .iter()
            .filter_map(|l| {
                let model = models.get(l).or_else(|| models.get("*"))?;
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

    /// What one engine process needs when it loads every model in `model_ids` at once (a batch engine holds one
    /// model for each distinct id of the job's `models_by_lang`: English on the rolling model and Hindi on the night
    /// model are two models in one process). Each distinct model counts its own measured peak, so the sum is a
    /// little above the true peak (the process's fixed part is counted once per model) and never below it.
    /// The second smoke measured 4095 MiB for turbo q5_0 plus large-v3 q5_0 against 2780 + 1526 = 4306 MiB here;
    /// taking the larger of the two (2780) put the engine at 73.6% of its memory limit.
    pub fn memory_for_models<'a>(
        &self,
        model_ids: impl IntoIterator<Item = &'a str>,
        gpu: bool,
    ) -> (u64, u64) {
        let distinct: std::collections::BTreeSet<&str> = model_ids.into_iter().collect();
        distinct
            .into_iter()
            .fold((0u64, 0u64), |(peak, local), id| {
                let (p, l) = self.peak_for(id, gpu);
                (peak.saturating_add(p), local.saturating_add(l))
            })
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

    /// The real-time factor the profile measured for `model` on clips of `lang` with the queue's (rolling)
    /// settings on a kind of device, or, when there is no such cell, the slowest one of the model there
    /// (`rtf_for`). 0 when nothing was measured.
    pub fn rtf_for_lang(&self, model: &str, lang: &str, gpu: bool) -> f64 {
        let exact = self
            .peaks
            .iter()
            .filter(|p| p.rolling && p.gpu == gpu && p.model == model && p.lang == lang)
            .map(|p| p.rtf)
            .fold(0.0, f64::max);
        if exact > 0.0 {
            exact
        } else {
            self.rtf_for(model, gpu)
        }
    }

    /// The profile's real-time factor for the audio of one unit: each language's measured factor for the model
    /// `models` (the job's `models_by_lang`, with `*` for any other language) gives it, weighted by `mix_ms`, the
    /// milliseconds of speech in each language. `None` when the mix is empty or a language has no model or no
    /// measurement.
    pub fn expected_rtf(
        &self,
        models: &serde_json::Map<String, serde_json::Value>,
        mix_ms: &BTreeMap<String, u64>,
        gpu: bool,
    ) -> Option<f64> {
        let total: u64 = mix_ms.values().sum();
        if total == 0 {
            return None;
        }
        let mut weighted = 0.0;
        for (lang, ms) in mix_ms {
            let id = models
                .get(lang)
                .or_else(|| models.get("*"))
                .and_then(|v| v.as_str())?;
            let rtf = self.rtf_for_lang(id, lang, gpu);
            if rtf <= 0.0 {
                return None;
            }
            weighted += rtf * *ms as f64;
        }
        Some(weighted / total as f64)
    }

    /// The profile's real-time factor for each of `languages` with the model `models` gives it (`*` for a
    /// language without its own); a language with no model or no measurement is left out.
    pub fn rtf_by_lang(
        &self,
        models: &serde_json::Map<String, serde_json::Value>,
        languages: &[String],
        gpu: bool,
    ) -> BTreeMap<String, f64> {
        languages
            .iter()
            .filter_map(|l| {
                let id = models
                    .get(l)
                    .or_else(|| models.get("*"))
                    .and_then(|v| v.as_str())?;
                let rtf = self.rtf_for_lang(id, l, gpu);
                (rtf > 0.0).then(|| (l.clone(), rtf))
            })
            .collect()
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
    fn english_and_hindi_sessions_use_the_hinglish_model_only_where_the_check_chose_it() {
        use wb_capability::Hinglish;
        let langs: Vec<String> = vec!["en".into(), "hi".into()];
        // Not measured here (or too slow): today's per-language picks stay.
        let mut cap = tier_g();
        cap.decision.hinglish = Hinglish {
            ok: false,
            rtf: Some(1.4),
            source: "measured".into(),
            model: Some("hinglish-f16".into()),
            night_ok: false,
            night_hours: None,
        };
        let kept = ProfileView::from_capability(&cap, vec![]);
        assert!(!kept.uses_hinglish(&langs));
        assert!(
            !kept
                .rolling_models(&langs)
                .values()
                .any(|m| m == "hinglish-f16")
        );
        // The proxy never chooses it: `ok` without its own cell has no model.
        cap.decision.hinglish = Hinglish {
            ok: true,
            rtf: Some(0.4),
            source: "proxy: large-v3-q5_0 (same size)".into(),
            model: None,
            night_ok: false,
            night_hours: None,
        };
        assert!(!ProfileView::from_capability(&cap, vec![]).uses_hinglish(&langs));
        // Measured and within the bar: every chunk of an English + Hindi session, both passes, one model.
        cap.decision.hinglish = Hinglish {
            ok: true,
            rtf: Some(0.4),
            source: "measured".into(),
            model: Some("hinglish-f16".into()),
            night_ok: false,
            night_hours: None,
        };
        let v = ProfileView::from_capability(&cap, vec![]);
        assert!(v.uses_hinglish(&langs));
        let rolling = v.rolling_models(&langs);
        assert_eq!(rolling.len(), 3, "{rolling:?}");
        assert!(rolling.values().all(|m| m == "hinglish-f16"), "{rolling:?}");
        assert_eq!(v.night_models(&langs), rolling, "no night model beats it");
        // English only, Hindi only and English + Urdu keep their own picks.
        for other in [
            vec!["en".to_string()],
            vec!["hi".to_string()],
            vec!["en".to_string(), "ur".to_string()],
        ] {
            assert!(!v.uses_hinglish(&other), "{other:?}");
            assert!(
                !v.rolling_models(&other)
                    .values()
                    .any(|m| m == "hinglish-f16"),
                "{other:?}"
            );
        }
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
    fn an_engine_that_loads_two_models_counts_both_peaks_and_a_repeated_model_once() {
        let cap = tier_g();
        let v = ProfileView::from_capability(&cap, vec![]);
        let mut ids: Vec<String> = cap
            .profile
            .measurements
            .iter()
            .map(|m| m.model.clone())
            .collect();
        ids.sort();
        ids.dedup();
        assert!(ids.len() >= 2, "the fixture needs two measured models");
        let (a, b) = (ids[0].as_str(), ids[1].as_str());
        for gpu in [false, true] {
            let (pa, la) = v.peak_for(a, gpu);
            let (pb, lb) = v.peak_for(b, gpu);
            assert_eq!(v.memory_for_models([a, b, a], gpu), (pa + pb, la + lb));
            assert_eq!(v.memory_for_models([a, a, a], gpu), (pa, la));
            assert_eq!(v.memory_for_models([b], gpu), (pb, lb));
        }
        assert_eq!(v.memory_for_models([], false), (0, 0));
    }

    /// The second smoke (emu-low, 17 Nov 2026): the night engine held turbo q5_0 (English) and large-v3 q5_0
    /// (Hindi) and peaked at 4095 MiB. Sized on the larger model alone (2780 MiB) the limit was 5561 MiB and the
    /// engine sat at 73.6% of it, over the 70% bar; sized on both models it is far under.
    #[test]
    fn the_second_smokes_night_engine_is_under_the_safe_share_of_a_limit_sized_on_both_models() {
        use wb_queue::sched::{engine_memory_limit, over_safe_share};
        const MIB: u64 = 1 << 20;
        let mut cap = tier_g();
        let models = ["large-v3-turbo-q5_0", "large-v3-q5_0"];
        let peaks = [1526 * MIB, 2780 * MIB];
        cap.profile.measurements.truncate(2);
        for (m, (id, peak)) in cap
            .profile
            .measurements
            .iter_mut()
            .zip(models.iter().zip(peaks))
        {
            m.model = (*id).to_string();
            m.peak_commit_bytes = peak;
            m.config = wb_capability::DeviceConfig::Gpu { index: 0 };
        }
        let v = ProfileView::from_capability(&cap, vec![]);
        let measured_engine_peak = 4095 * MIB;
        let ample = 11 * (1 << 30);
        let old = engine_memory_limit(v.peak_for(models[1], true).0, ample);
        assert!(
            over_safe_share(measured_engine_peak, old),
            "the old sizing was over the bar"
        );
        let (peak, _) = v.memory_for_models(models, true);
        assert_eq!(peak, (1526 + 2780) * MIB);
        assert!(
            peak >= measured_engine_peak,
            "the sum never undercounts here"
        );
        let new = engine_memory_limit(peak, ample);
        assert!(!over_safe_share(measured_engine_peak, new));
    }

    /// The second smoke's profile cells (emu-low, Vulkan UHD, 2 threads), as the speed log compares units with.
    fn smoke_cells() -> ProfileView {
        use wb_capability::{DeviceConfig, Lang, Mode};
        let mut cap = tier_g();
        while cap.profile.measurements.len() < 3 {
            let first = cap.profile.measurements[0].clone();
            cap.profile.measurements.push(first);
        }
        cap.profile.measurements.truncate(3);
        let cells = [
            ("large-v3-turbo-q5_0", Lang::En, 0.389),
            ("large-v3-turbo-q5_0", Lang::Hi, 0.696),
            ("large-v3-q5_0", Lang::Hi, 1.479),
        ];
        for (m, (id, lang, rtf)) in cap.profile.measurements.iter_mut().zip(cells) {
            m.model = id.to_string();
            m.lang = lang;
            m.rtf = rtf;
            m.mode = Mode::Rolling;
            m.config = DeviceConfig::Gpu { index: 0 };
        }
        ProfileView::from_capability(&cap, vec![])
    }

    #[test]
    fn the_expected_speed_of_a_unit_weights_each_languages_cell_by_its_share_of_the_speech() {
        let v = smoke_cells();
        let models = serde_json::json!({
            "*": "large-v3-q5_0", "en": "large-v3-turbo-q5_0", "hi": "large-v3-q5_0"
        });
        let models = models.as_object().unwrap();
        // A night unit that is 10% English (turbo, 0.389) and 90% Hindi (large-v3 q5_0, 1.479).
        let mix = BTreeMap::from([("en".to_string(), 20_000u64), ("hi".to_string(), 180_000)]);
        let e = v.expected_rtf(models, &mix, true).unwrap();
        assert!((e - (0.389 * 0.1 + 1.479 * 0.9)).abs() < 1e-9, "{e}");
        // A language the job has only the `*` model for takes it; the cell of the model's slowest language stands in.
        let ur = BTreeMap::from([("ur".to_string(), 60_000u64)]);
        let e = v.expected_rtf(models, &ur, true).unwrap();
        assert!((e - 1.479).abs() < 1e-9, "{e}");
        // Nothing measured on the processor, nothing to compare with.
        assert_eq!(v.expected_rtf(models, &mix, false), None);
        // No speech, no figure.
        assert_eq!(v.expected_rtf(models, &BTreeMap::new(), true), None);
        // The cell of the language itself, not the model's slowest one.
        assert!((v.rtf_for_lang("large-v3-turbo-q5_0", "en", true) - 0.389).abs() < 1e-9);
        assert!((v.rtf_for_lang("large-v3-turbo-q5_0", "hi", true) - 0.696).abs() < 1e-9);
        assert!((v.rtf_for_lang("large-v3-turbo-q5_0", "ur", true) - 0.696).abs() < 1e-9);
    }

    #[test]
    fn the_night_plan_counts_english_at_the_rolling_model_and_hindi_at_the_night_model() {
        let mut v = smoke_cells();
        // English has no night model of its own here (the shape of the 21 Nov run), Hindi has one.
        v.night.remove("en");
        v.night
            .insert("hi".to_string(), "large-v3-q5_0".to_string());
        let langs = vec!["en".to_string(), "hi".to_string()];
        let night = v.night_rtf(&langs);
        let decode = v.night_decode_rtf(&langs);
        // Only Hindi has a night model, so only Hindi decides that a session gets a night job...
        assert_eq!(
            night.keys().cloned().collect::<Vec<_>>(),
            vec!["hi".to_string()]
        );
        // ...but the job decodes the whole session, so the plan counts English too, at the rolling model's speed.
        assert_eq!(decode.keys().cloned().collect::<Vec<_>>(), langs);
        assert!((decode["hi"] - night["hi"]).abs() < 1e-9);
        let en_model = v.night_models(&langs)["en"].clone();
        assert!((decode["en"] - v.rtf_for(&en_model, true)).abs() < 1e-9);
    }

    #[test]
    fn a_session_without_a_night_model_gets_no_night_pass() {
        let v = ProfileView::from_capability(&tier_g(), vec![]);
        assert!(v.night_rtf(&["xx".into()]).is_empty());
    }
}
