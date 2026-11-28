//! The scheduler's running state and the gate it gives every engine process: pause rules from
//! `wb_queue::sched::Scheduler`, the 3 GB start reserve, the per-engine memory limit. Everything the machine
//! tells it comes through [`Readings`], so the rules run in tests without Windows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use wb_queue::sched::{Scheduler, Snapshot, Transition, Verdict, engine_memory_limit, start_gate};
use wb_runner::gate::{Grant, StartGate, StartRequest, Wait};

use super::profile::ProfileView;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the machine and the recorder say right now.
pub trait Readings: Send + Sync {
    fn now_ms(&self) -> u64;
    /// Free RAM as the machine view reports it (host free memory, or the job's remaining limit under
    /// emulation), with the graphics memory the engines hold counted.
    fn free_ram_bytes(&self) -> u64;
    fn recording(&self) -> bool;
    fn on_battery(&self) -> bool;
    /// Running total of the recorder's overruns, xruns and dropped samples.
    fn recorder_faults(&self) -> u64;
    /// The capability profile in use, read fresh (a new check or a fingerprint change shows at once).
    fn profile(&self) -> Option<ProfileView>;
}

/// One line of the scheduler log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub at_ms: u64,
    pub text: String,
}

/// The queue's view of the scheduler: the latest verdict, the log and the operator's pause.
pub struct Core {
    readings: Arc<dyn Readings>,
    scheduler: Mutex<Scheduler>,
    verdict: Mutex<Verdict>,
    operator_paused: AtomicBool,
    log: Mutex<Vec<LogEntry>>,
}

impl Core {
    pub fn new(readings: Arc<dyn Readings>) -> Self {
        Self {
            readings,
            scheduler: Mutex::new(Scheduler::new()),
            verdict: Mutex::new(Verdict {
                pause: None,
                fewer_threads: 0,
            }),
            operator_paused: AtomicBool::new(false),
            log: Mutex::new(Vec::new()),
        }
    }

    pub fn set_operator_paused(&self, paused: bool) {
        self.operator_paused.store(paused, Ordering::SeqCst);
    }

    pub fn operator_paused(&self) -> bool {
        self.operator_paused.load(Ordering::SeqCst)
    }

    pub fn verdict(&self) -> Verdict {
        *lock(&self.verdict)
    }

    pub fn log(&self) -> Vec<LogEntry> {
        lock(&self.log).clone()
    }

    pub fn push_log(&self, text: String) {
        #[cfg(feature = "gate-hooks")]
        super::gate_hooks::queue_log(&text);
        let at_ms = self.readings.now_ms();
        // The same line goes to `app.log` with the IST time it was written at (the Queue screen shows only the newest
        // lines, and a person checking the night reads this file, not a screenshot).
        tracing::info!(
            "scheduler: {} {}",
            wb_queue::estimate::format_ist_stamp(at_ms),
            text
        );
        let mut log = lock(&self.log);
        log.push(LogEntry { at_ms, text });
        // The app log keeps everything; the screen needs the recent lines.
        let excess = log.len().saturating_sub(500);
        if excess > 0 {
            log.drain(..excess);
        }
    }

    /// Re-reads the machine and decides. Returns the verdict and what changed since the last call (each change
    /// is also in the log with its reason).
    pub fn evaluate(&self) -> (Verdict, Vec<Transition>) {
        let profile = self.readings.profile();
        let snap = Snapshot {
            now_ms: self.readings.now_ms(),
            recording: self.readings.recording(),
            process_while_recording: profile.as_ref().is_some_and(|p| p.process_while_recording),
            // No profile at all (no check yet): nothing is known about this computer, so only record.
            record_only: profile.as_ref().is_none_or(|p| p.record_only),
            free_ram_bytes: self.readings.free_ram_bytes(),
            on_battery: self.readings.on_battery(),
            pause_on_battery: profile.as_ref().is_none_or(|p| p.pause_on_battery),
            recorder_faults: self.readings.recorder_faults(),
            live_active: false,
            cpu_only: false,
            operator_paused: self.operator_paused(),
        };
        let mut transitions = Vec::new();
        let verdict = lock(&self.scheduler).evaluate(&snap, &mut transitions);
        *lock(&self.verdict) = verdict;
        for t in &transitions {
            self.push_log(describe(t));
        }
        (verdict, transitions)
    }

    fn pause_text(&self) -> Option<String> {
        lock(&self.verdict).pause.map(|r| r.text().to_owned())
    }

    /// Free RAM now, for the Queue screen's scheduler card.
    pub fn free_ram_bytes(&self) -> u64 {
        self.readings.free_ram_bytes()
    }
}

fn describe(t: &Transition) -> String {
    match t {
        Transition::Paused(r) => format!("paused: {}", r.text()),
        Transition::Resumed(r) => format!("resumed (was: {})", r.text()),
        Transition::ThreadsLowered(n) => {
            format!(
                "a second overrun within ten minutes: {n} thread(s) fewer for the batch engines"
            )
        }
    }
}

/// The gate the manager and the planner use: a fresh [`Core::evaluate`] for every decision and the start reserve
/// from a fresh memory reading.
pub struct AppGate {
    pub core: Arc<Core>,
}

impl StartGate for AppGate {
    fn paused(&self) -> Option<String> {
        // Every decision reads the machine and the profile in use afresh (tier R, processing while recording,
        // the fingerprint's safe defaults); nothing is decided from an earlier reading.
        self.core.evaluate();
        self.core.pause_text()
    }

    fn check(&self, request: &StartRequest) -> Result<Grant, Wait> {
        self.core.evaluate();
        if let Some(reason) = self.core.pause_text() {
            return Err(Wait { reason });
        }
        let free = self.core.readings.free_ram_bytes();
        // What the engines already running may still grow by comes with the request (the manager tracks it).
        match start_gate(
            free,
            request.peak_commit_bytes,
            request.gpu_local_bytes,
            request.other_headroom_bytes,
        ) {
            Ok(()) => Ok(Grant {
                memory_limit_bytes: Some(engine_memory_limit(request.peak_commit_bytes, free)),
            }),
            Err(refused) => Err(Wait {
                reason: format!(
                    "starting needs {:.1} GB free after the engine's peak of {:.1} GB (3 GB stay free), {:.1} GB are free now",
                    refused.needed_bytes as f64 / f64::from(1u32 << 30),
                    request.peak_commit_bytes as f64 / f64::from(1u32 << 30),
                    free as f64 / f64::from(1u32 << 30),
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wb_queue::sched::PauseReason;

    const GIB: u64 = 1 << 30;

    struct Fake {
        now: AtomicU64,
        free: AtomicU64,
        recording: AtomicBool,
        battery: AtomicBool,
        faults: AtomicU64,
        profile: Mutex<Option<ProfileView>>,
    }

    impl Fake {
        fn new(profile: Option<ProfileView>) -> Arc<Self> {
            Arc::new(Self {
                now: AtomicU64::new(1_000),
                free: AtomicU64::new(8 * GIB),
                recording: AtomicBool::new(false),
                battery: AtomicBool::new(false),
                faults: AtomicU64::new(0),
                profile: Mutex::new(profile),
            })
        }
    }

    impl Readings for Fake {
        fn now_ms(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
        fn free_ram_bytes(&self) -> u64 {
            self.free.load(Ordering::SeqCst)
        }
        fn recording(&self) -> bool {
            self.recording.load(Ordering::SeqCst)
        }
        fn on_battery(&self) -> bool {
            self.battery.load(Ordering::SeqCst)
        }
        fn recorder_faults(&self) -> u64 {
            self.faults.load(Ordering::SeqCst)
        }
        fn profile(&self) -> Option<ProfileView> {
            lock(&self.profile).clone()
        }
    }

    fn cap(json: &str) -> ProfileView {
        ProfileView::from_capability(&wb_capability::parse_capability(json).unwrap(), vec![])
    }

    fn tier_g() -> ProfileView {
        cap(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ))
    }

    fn req(peak: u64) -> StartRequest {
        StartRequest {
            what: "unit".into(),
            peak_commit_bytes: peak,
            gpu_local_bytes: 0,
            other_headroom_bytes: 0,
        }
    }

    fn gate(readings: &Arc<Fake>) -> (Arc<Core>, AppGate) {
        let core = Arc::new(Core::new(readings.clone()));
        core.evaluate();
        let g = AppGate { core: core.clone() };
        (core, g)
    }

    #[test]
    fn tier_r_processes_nothing_at_all_chunks_included() {
        let fake = Fake::new(Some(cap(include_str!(
            "../../../crates/wb-capability/fixtures/tier-r.json"
        ))));
        let (_core, g) = gate(&fake);
        assert!(g.paused().unwrap().contains("only records"));
        let refused = g.check(&req(GIB)).unwrap_err();
        assert!(refused.reason.contains("only records"), "{refused:?}");
    }

    #[test]
    fn with_no_profile_nothing_is_processed() {
        let fake = Fake::new(None);
        let (_core, g) = gate(&fake);
        assert!(g.paused().is_some());
    }

    #[test]
    fn processing_while_recording_is_refused_where_the_profile_refuses_it() {
        let mut p = tier_g();
        p.process_while_recording = false;
        let fake = Fake::new(Some(p));
        fake.recording.store(true, Ordering::SeqCst);
        let (core, g) = gate(&fake);
        let (v, _) = core.evaluate();
        assert_eq!(v.pause, Some(PauseReason::RecordingRefused));
        assert!(
            g.check(&req(GIB))
                .unwrap_err()
                .reason
                .contains("while recording")
        );
        // Stop: processing may start, and the reason change is in the log.
        fake.recording.store(false, Ordering::SeqCst);
        let (v, _) = core.evaluate();
        assert_eq!(v.pause, None);
        assert!(core.log().iter().any(|l| l.text.starts_with("resumed")));
        assert!(g.check(&req(GIB)).is_ok());
    }

    #[test]
    fn the_start_gate_keeps_three_gigabytes_free_and_grants_a_limit() {
        let mut p = tier_g();
        p.process_while_recording = true;
        let fake = Fake::new(Some(p));
        let (_core, g) = gate(&fake);
        // 8 GB free, peak 4 GB: 4 GB left, at least 3 GB stay: allowed, with a limit of max(8, 5) GB capped by
        // free - 2 GB = 6 GB.
        let grant = g.check(&req(4 * GIB)).unwrap();
        assert_eq!(grant.memory_limit_bytes, Some(6 * GIB));
        // Peak 6 GB: only 2 GB would stay free: refused, and the reason says how much is needed.
        let refused = g.check(&req(6 * GIB)).unwrap_err();
        assert!(refused.reason.contains("3 GB stay free"), "{refused:?}");
    }

    #[test]
    fn every_decision_reads_the_profile_in_use_afresh() {
        // The profile starts as a computer that may process while recording; nobody calls `evaluate` between the
        // changes below: the gate itself must see each one at once (a new check, a fingerprint change, a rule).
        let mut p = tier_g();
        p.process_while_recording = true;
        let fake = Fake::new(Some(p.clone()));
        fake.recording.store(true, Ordering::SeqCst);
        let (_core, g) = gate(&fake);
        assert!(g.paused().is_none());
        assert!(g.check(&req(GIB)).is_ok());

        // process_while_recording turns false (a new check says so): the very next decision refuses.
        let mut refusing = p.clone();
        refusing.process_while_recording = false;
        *lock(&fake.profile) = Some(refusing);
        assert!(g.paused().unwrap().contains("while recording"));
        assert!(
            g.check(&req(GIB))
                .unwrap_err()
                .reason
                .contains("while recording")
        );

        // Back to allowed, then tier R, then a fingerprint change (the profile's safe defaults refuse recording
        // work), then no profile at all: each takes effect at the next decision, without any evaluate call.
        *lock(&fake.profile) = Some(p.clone());
        assert!(g.paused().is_none());
        *lock(&fake.profile) = Some(cap(include_str!(
            "../../../crates/wb-capability/fixtures/tier-r.json"
        )));
        assert!(g.paused().unwrap().contains("only records"));
        assert!(g.check(&req(GIB)).is_err());
        let mut changed = p.clone();
        changed.process_while_recording = false;
        changed.changes = vec!["the graphics driver changed".into()];
        *lock(&fake.profile) = Some(changed);
        assert!(g.paused().is_some());
        *lock(&fake.profile) = None;
        assert!(g.paused().is_some());
        *lock(&fake.profile) = Some(p);
        assert!(g.paused().is_none());
    }

    #[test]
    fn a_session_started_before_the_engine_scan_ends_keeps_its_job_and_is_processed_once_it_ends() {
        // Before the engine scan has finished the app has no fingerprint, so the saved profile is read as
        // `unidentified`: safe defaults (no processing while recording), models and configuration as measured.
        let saved = wb_capability::parse_capability(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ))
        .unwrap();
        let mut identified = ProfileView::from_capability(&saved, vec![]);
        identified.process_while_recording = true;
        let unidentified = ProfileView::from_capability(
            &wb_capability::safe_defaults(&saved, "this computer has not been identified yet"),
            vec!["this computer has not been identified yet".into()],
        );
        assert!(!unidentified.process_while_recording);

        // The session's rolling job is made from what the unidentified profile says: the same models and the
        // same engine configuration as the identified one, so nothing about the job waits for the scan.
        let langs = ["en".to_owned()];
        assert!(!unidentified.rolling_models(&langs).is_empty());
        assert_eq!(
            unidentified.rolling_models(&langs),
            identified.rolling_models(&langs)
        );
        assert_eq!(unidentified.config, identified.config);

        // The session is recording and the profile is still unidentified: the rules hold the queue back ...
        let fake = Fake::new(Some(unidentified));
        fake.recording.store(true, Ordering::SeqCst);
        let (_core, g) = gate(&fake);
        assert!(g.paused().unwrap().contains("while recording"));
        assert!(g.check(&req(GIB)).is_err());

        // ... and when the scan finishes and the profile is identified, the very next decision lets the job
        // run, with no evaluate call in between and the session still recording: processing the profile allows
        // is not lost for the rest of the session.
        *lock(&fake.profile) = Some(identified);
        assert!(g.paused().is_none());
        assert!(g.check(&req(GIB)).is_ok());
    }

    #[test]
    fn free_memory_under_two_gigabytes_pauses_and_it_resumes_above_three() {
        let mut p = tier_g();
        p.process_while_recording = true;
        let fake = Fake::new(Some(p));
        let (core, g) = gate(&fake);
        fake.free.store(GIB + GIB / 2, Ordering::SeqCst);
        let (v, _) = core.evaluate();
        assert_eq!(v.pause, Some(PauseReason::LowMemory));
        assert!(g.paused().is_some());
        fake.free.store(2 * GIB + GIB / 2, Ordering::SeqCst);
        assert!(
            core.evaluate().0.pause.is_some(),
            "between 2 and 3 GB it stays paused"
        );
        fake.free.store(4 * GIB, Ordering::SeqCst);
        assert!(core.evaluate().0.pause.is_none());
    }

    #[test]
    fn a_second_headroom_is_counted_for_the_second_engine() {
        let fake = Fake::new(Some(tier_g()));
        let (_core, g) = gate(&fake);
        let with = |peak: u64| StartRequest {
            other_headroom_bytes: 2 * GIB,
            ..req(peak)
        };
        // 8 GB free, peak 3 GB, the other engine may still grow 2 GB: 3 GB left: just allowed.
        assert!(g.check(&with(3 * GIB)).is_ok());
        assert!(g.check(&with(3 * GIB + 1)).is_err());
    }

    /// Integrated graphics memory is system RAM: an engine on it counts its local memory like commit.
    #[test]
    fn graphics_memory_in_system_ram_counts_against_the_start_reserve() {
        let fake = Fake::new(Some(tier_g()));
        let (_core, g) = gate(&fake);
        let with_gpu = |peak: u64, local: u64| StartRequest {
            gpu_local_bytes: local,
            ..req(peak)
        };
        // 8 GB free, 2 GB commit + 3 GB graphics memory leaves 3 GB: allowed; a byte more is refused.
        assert!(g.check(&with_gpu(2 * GIB, 3 * GIB)).is_ok());
        assert!(g.check(&with_gpu(2 * GIB, 3 * GIB + 1)).is_err());
    }

    #[test]
    fn the_operator_pause_holds_every_start() {
        let fake = Fake::new(Some(tier_g()));
        let (core, g) = gate(&fake);
        core.set_operator_paused(true);
        core.evaluate();
        assert!(g.paused().unwrap().contains("operator"));
        core.set_operator_paused(false);
        core.evaluate();
        assert!(g.paused().is_none());
    }

    /// A writer the test reads back: what `tracing` would write to `app.log`.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// 28 Nov 2026: the scheduler's lines were kept in memory only, and the Queue screen shows the newest eight, so a person
    /// checking the night's switches had nothing to read afterwards. Every line is also written to `app.log`, stamped with the
    /// IST time it was written at.
    #[test]
    fn every_queue_log_line_is_also_written_to_the_app_log_with_its_ist_time() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_target(false)
            .finish();
        let fake = Fake::new(Some(tier_g()));
        // 2026-11-28 20:05:13 IST is 14:35:13 UTC.
        let at_ms = 1_795_876_513_000u64;
        fake.now.store(at_ms, Ordering::SeqCst);
        let (core, _g) = gate(&fake);
        tracing::subscriber::with_default(subscriber, || {
            core.push_log("night safety switch: S3 is dropped from the night".into());
            core.push_log("night job of S1 is held".into());
        });
        let written = String::from_utf8(lock(&captured.0).clone()).unwrap();
        let stamp = wb_queue::estimate::format_ist_stamp(at_ms);
        assert!(stamp.ends_with("IST") && stamp.contains(':'), "{stamp}");
        assert!(
            written.contains(&format!(
                "scheduler: {stamp} night safety switch: S3 is dropped from the night"
            )),
            "{written}"
        );
        assert!(
            written.contains(&format!("scheduler: {stamp} night job of S1 is held")),
            "{written}"
        );
        // The screen's memory still has the lines too.
        assert!(
            core.log()
                .iter()
                .any(|l| l.text.starts_with("night job of S1"))
        );
    }
}
