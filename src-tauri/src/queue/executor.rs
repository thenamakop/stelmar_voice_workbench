//! Runs a claimed unit on a real engine process: finds the engine build for the unit's device, resolves the
//! job's model ids to verified model files, builds the batch request and hands it to the supervisor
//! (`wb_runner::exec::run_unit`: suspended start, a Job Object of its own, a memory limit, watchdog, restarts
//! from the staged file's last chunk marker). The process runs at below-normal priority.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use wb_models::{Cache, Manifest};
use wb_protocol::{ModelRef, Settings};
use wb_queue::store::Device;
use wb_runner::exec::{UnitContext, UnitEnd, UnitExecutor, run_unit};
use wb_runner::gate::Grant;
use wb_runner::session_queue::Claim;
use wb_supervisor::{Cancel, Observer, Policy, Priority};

use super::profile::ProfileView;

/// The id of the Silero VAD model in `models/manifest.json`.
pub const VAD_ID: &str = "silero-v6.2.0";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the app knows about the engines and models of this installation, and how the profile in use sets the
/// engines up.
pub struct Environment {
    pub models_dir: PathBuf,
    pub manifest: Manifest,
    /// Engine executable per variant (`avx2`, `avx512`, `vulkan`, `cuda`), from `engines::discover`.
    pub engines: Vec<(String, PathBuf)>,
    /// Only the development and gate builds pass `WB_*` variables on (emulation needs them).
    pub scrub_env: bool,
    pub profile: Arc<dyn Fn() -> Option<ProfileView> + Send + Sync>,
    /// Session folders by session name, for the glossary, the language list and the lock log.
    pub contexts: SessionLookup,
}

/// Finds an open session's folder, glossary and languages by name.
pub type SessionLookup = Arc<dyn Fn(&str) -> Option<SessionInfoForUnits> + Send + Sync>;

/// What a unit needs to know about its session beyond the job.
#[derive(Debug, Clone)]
pub struct SessionInfoForUnits {
    pub dir: PathBuf,
    pub glossary: String,
    pub languages: Vec<String>,
}

/// Four times the time the measured speed needs for the longest chunk (15 s), never under 60 s.
pub fn watchdog_for(rtf: f64) -> Duration {
    let wanted = if rtf.is_finite() && rtf > 0.0 {
        4.0 * 15.0 * rtf
    } else {
        0.0
    };
    Duration::from_secs_f64(wanted.max(60.0))
}

/// How the app starts every batch engine: below-normal priority, a watchdog from the measured speed, and the
/// environment scrubbed of `WB_*` and the device-selection variables where `scrub_env` says so (row 102).
pub fn engine_policy(scrub_env: bool, rtf: f64) -> Policy {
    Policy {
        watchdog: watchdog_for(rtf),
        priority: Priority::BelowNormal,
        scrub_env,
        ..Policy::default()
    }
}

pub struct AppExecutor {
    env: Environment,
    cache: Mutex<Cache>,
    /// Batch threads currently allowed (the scheduler lowers them after a second overrun).
    threads_fewer: std::sync::atomic::AtomicU32,
}

impl AppExecutor {
    pub fn new(env: Environment) -> Self {
        Self {
            env,
            cache: Mutex::new(Cache::default()),
            threads_fewer: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// The scheduler's "fewer threads" for the batch engines (a second overrun within ten minutes).
    pub fn set_threads_fewer(&self, n: u32) {
        self.threads_fewer
            .store(n, std::sync::atomic::Ordering::SeqCst);
    }

    fn engine_exe(&self, device: Device, profile: &ProfileView) -> Option<(String, PathBuf)> {
        let want: &[&str] = match device {
            Device::Cpu => &["avx512", "avx2"],
            Device::Gpu => &["vulkan", "cuda"],
        };
        // The build the profile measured for this kind of device comes first.
        let measured = profile.engine_build(device == Device::Gpu);
        measured
            .into_iter()
            .chain(want.iter().copied())
            .find_map(|v| {
                self.env
                    .engines
                    .iter()
                    .find(|(name, _)| name == v)
                    .map(|(n, p)| (n.clone(), p.clone()))
            })
    }

    fn model_ref(&self, id: &str) -> Result<ModelRef, String> {
        let mut cache = lock(&self.cache);
        let path =
            wb_models::path_for_engine(&self.env.models_dir, &self.env.manifest, id, &mut cache)?;
        Ok(ModelRef {
            id: id.to_owned(),
            path: path.to_string_lossy().into_owned(),
        })
    }

    fn context(
        &self,
        claim: &Claim,
        profile: &ProfileView,
        info: &SessionInfoForUnits,
    ) -> Result<UnitContext, String> {
        let mut models_by_lang: BTreeMap<String, ModelRef> = BTreeMap::new();
        if let Some(map) = claim.job.models_by_lang.as_object() {
            for (lang, id) in map {
                if let Some(id) = id.as_str() {
                    models_by_lang.insert(lang.clone(), self.model_ref(id)?);
                }
            }
        }
        if models_by_lang.is_empty() {
            return Err("the job has no model for any language".into());
        }
        let gpu = claim.device == Device::Gpu;
        let base = profile.cpu_threads.max(1);
        let fewer = self.threads_fewer.load(std::sync::atomic::Ordering::SeqCst);
        let mut threads = base.saturating_sub(fewer).max(1);
        if gpu {
            // A graphics engine needs few processor threads (its own decoder feed).
            threads = threads.min(2);
        }
        let role = match claim.job.kind {
            wb_queue::store::JobKind::Night => "night",
            _ => "rolling",
        };
        Ok(UnitContext {
            session_dir: info.dir.clone(),
            models_by_lang,
            vad_model: self.model_ref(VAD_ID)?,
            languages: if info.languages.is_empty() {
                claim.job.languages.clone()
            } else {
                info.languages.clone()
            },
            lang_log: super::langlock::read(&info.dir),
            settings: Settings {
                role: role.into(),
                glossary: info.glossary.clone(),
                ..Settings::default()
            },
            threads,
            gpu_index: if gpu {
                profile.gpu_index.or(Some(0))
            } else {
                None
            },
        })
    }
}

impl UnitExecutor for AppExecutor {
    fn expected_memory(&self, claim: &Claim) -> (u64, u64) {
        let Some(profile) = (self.env.profile)() else {
            return (u64::MAX / 4, 0);
        };
        let gpu = claim.device == Device::Gpu;
        // The engine loads every distinct model of the job (up to two: the night pass can have the rolling model for
        // English and a large model for Hindi), so their peaks add up (found by the second smoke: 73.6% of a limit
        // that was sized on the larger model alone).
        let (peak, local) = match claim.job.models_by_lang.as_object() {
            Some(map) => profile
                .memory_for_models(map.values().filter_map(|v| v.as_str()), gpu),
            None => (0, 0),
        };
        if peak == 0 {
            // Nothing measured at all: assume the largest the profile holds, else a figure that refuses to
            // start rather than guessing small (AGENTS.md: measured, not assumed).
            let (p, l) = profile.peak_for("", gpu);
            return (if p == 0 { 6 << 30 } else { p }, l);
        }
        (peak, local)
    }

    fn expected_rtf(&self, claim: &Claim, mix_ms: &BTreeMap<String, u64>) -> Option<f64> {
        let profile = (self.env.profile)()?;
        let models = claim.job.models_by_lang.as_object()?;
        profile.expected_rtf(models, mix_ms, claim.device == Device::Gpu)
    }

    fn run(
        &self,
        claim: &Claim,
        grant: Grant,
        cancel: &Cancel,
        observer: &mut dyn Observer,
    ) -> UnitEnd {
        let Some(profile) = (self.env.profile)() else {
            return UnitEnd::Failed(
                "no capability profile: nothing is known about this computer".into(),
            );
        };
        let Some((variant, exe)) = self.engine_exe(claim.device, &profile) else {
            return UnitEnd::Failed(format!(
                "no {} engine build is installed",
                if claim.device == Device::Gpu {
                    "graphics"
                } else {
                    "processor"
                }
            ));
        };
        let Some(info) = (self.env.contexts)(&claim.job.session) else {
            return UnitEnd::Failed(format!("session {} is not open", claim.job.session));
        };
        let ctx = match self.context(claim, &profile, &info) {
            Ok(c) => c,
            Err(e) => return UnitEnd::Failed(e),
        };
        // Watchdog: the engine reports every chunk; a chunk is at most 15 s of audio, so four times the time the
        // slowest measured speed of the job's models needs for one, never under 60 s (closes the row 105 (c) note).
        let gpu = claim.device == Device::Gpu;
        let rtf = claim
            .job
            .models_by_lang
            .as_object()
            .map(|m| {
                m.values()
                    .filter_map(|v| v.as_str())
                    .map(|id| profile.rtf_for(id, gpu))
                    .fold(0.0, f64::max)
            })
            .unwrap_or(0.0);
        let policy = engine_policy(self.env.scrub_env, rtf);
        tracing::info!(
            unit = claim.unit.idx,
            job = claim.job.id,
            engine = variant,
            threads = ctx.threads,
            limit_bytes = grant.memory_limit_bytes.unwrap_or(0),
            "engine starting for a unit"
        );
        run_unit(&exe, claim, &ctx, grant, policy, cancel, observer)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// Row 102: the app's engines get a scrubbed environment in an installed (release) build, and keep `WB_EMULATE`
    /// and the device variables in gate and debug builds, which pass them on purpose.
    #[test]
    fn the_engine_policy_scrubs_wb_variables_in_a_release_build_and_keeps_them_in_gate_and_debug_builds()
     {
        let policy = engine_policy(crate::engines::scrub_engine_env(), 1.0);
        assert_eq!(policy.scrub_env, crate::engines::scrub_engine_env());
        #[cfg(any(debug_assertions, feature = "gate-hooks"))]
        assert!(
            !policy.scrub_env,
            "a gate or debug build must keep WB_EMULATE"
        );
        #[cfg(not(any(debug_assertions, feature = "gate-hooks")))]
        assert!(policy.scrub_env, "an installed build must scrub WB_*");
        assert_eq!(policy.priority, Priority::BelowNormal);
    }

    /// The VAD model every unit asks for must be in the committed manifest (a wrong id would fail every unit).
    #[test]
    fn the_vad_model_id_is_in_the_committed_manifest() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models")
            .join("manifest.json");
        let manifest = Manifest::load(&path).unwrap();
        assert!(
            manifest.find(VAD_ID).is_ok(),
            "{VAD_ID} is not in models/manifest.json"
        );
    }

    #[test]
    fn the_watchdog_follows_the_measured_speed_and_never_goes_under_a_minute() {
        assert_eq!(watchdog_for(0.0), Duration::from_secs(60));
        assert_eq!(watchdog_for(0.5), Duration::from_secs(60));
        assert_eq!(watchdog_for(2.0), Duration::from_secs(120));
        assert_eq!(watchdog_for(f64::NAN), Duration::from_secs(60));
    }
}
