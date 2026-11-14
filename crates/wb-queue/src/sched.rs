//! Scheduling rules (PLAN section 4, Phase 4 step 6) as pure functions of a resource snapshot, so every rule
//! is a unit test and the app's workers only apply the answer.
//!
//! Priority: recording > live > rolling > night > translation. The recorder is never touched: when memory or
//! CPU runs short the queue pauses, never the recorder (AGENTS.md non-negotiable 2).

use crate::store::{Job, JobKind, JobState};

pub const GIB: u64 = 1 << 30;
/// Never start an engine if free RAM after its measured peak would fall under this (AGENTS.md).
pub const START_RESERVE_BYTES: u64 = 3 * GIB;
/// Pause the batch engines when free RAM drops under this (AGENTS.md).
pub const PAUSE_BELOW_BYTES: u64 = 2 * GIB;
/// After a RAM pause, resume only above the start reserve again (a margin, so it does not flap).
pub const RESUME_ABOVE_BYTES: u64 = 3 * GIB;
/// An overrun pauses the queue; it resumes after this long with no new overrun.
pub const OVERRUN_CLEAR_MS: u64 = 60_000;
/// A second overrun within this window lowers the batch threads by one (PLAN, "If blocked").
pub const OVERRUN_REPEAT_WINDOW_MS: u64 = 600_000;

/// Why the queue is not processing. Every pause and resume is logged with its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// The operator pressed Pause.
    Operator,
    /// Tier R (no AVX2, under 8 GB, no validated model fits): nothing is processed here.
    RecordOnly,
    /// Recording is running and the profile refuses processing while recording (no passed recorder safety
    /// run, or the profile is stale after a fingerprint change).
    RecordingRefused,
    /// Free RAM, shared GPU memory counted, is under 2 GB.
    LowMemory,
    /// On battery and the operator has not overridden the rule.
    OnBattery,
    /// The recorder reported an overrun, xrun or dropped sample.
    RecorderOverrun,
    /// A live session on a CPU-only machine (Phase 7).
    LiveOnCpuOnly,
}

impl PauseReason {
    pub fn text(self) -> &'static str {
        match self {
            PauseReason::Operator => "paused by the operator",
            PauseReason::RecordOnly => "this computer only records",
            PauseReason::RecordingRefused => {
                "processing while recording is not cleared on this computer"
            }
            PauseReason::LowMemory => "free memory is under 2 GB",
            PauseReason::OnBattery => "running on battery",
            PauseReason::RecorderOverrun => "the recorder reported an overrun",
            PauseReason::LiveOnCpuOnly => "a live session is using the processor",
        }
    }
}

/// What the scheduler sees at one moment. The app fills it every 2 s (`wb_machine::probe()` for RAM, the
/// recorder status for overruns, the power status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub now_ms: u64,
    pub recording: bool,
    /// `Decision::process_while_recording` of the profile in use (`store::in_use`).
    pub process_while_recording: bool,
    pub record_only: bool,
    /// Free RAM as the machine view reports it, with the engines' GPU memory (shared or local) subtracted.
    pub free_ram_bytes: u64,
    pub on_battery: bool,
    /// The profile's `pause_on_battery` after the operator's override (false = keep going on battery).
    pub pause_on_battery: bool,
    /// Running totals of the recorder's overruns + xruns + dropped samples; a rise is a new overrun.
    pub recorder_faults: u64,
    pub live_active: bool,
    pub cpu_only: bool,
    pub operator_paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// `None` means the queue may run.
    pub pause: Option<PauseReason>,
    /// Batch threads to take off the profile's count (0 normally; 1 after repeated overruns).
    pub fewer_threads: u32,
}

/// One line of the scheduler log: a pause or a resume, with its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Paused(PauseReason),
    Resumed(PauseReason),
    ThreadsLowered(u32),
}

#[derive(Debug, Default)]
pub struct Scheduler {
    last_faults: Option<u64>,
    last_overrun_ms: Option<u64>,
    previous_overrun_ms: Option<u64>,
    low_memory: bool,
    fewer_threads: u32,
    current: Option<PauseReason>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decides for one snapshot. `log` receives every change (a pause, a resume, a lowered thread count).
    pub fn evaluate(&mut self, snap: &Snapshot, log: &mut Vec<Transition>) -> Verdict {
        // A rise in the recorder's fault counters is a new overrun.
        if let Some(last) = self.last_faults
            && snap.recorder_faults > last
        {
            if let Some(prev) = self.last_overrun_ms
                && snap.now_ms.saturating_sub(prev) <= OVERRUN_REPEAT_WINDOW_MS
            {
                self.previous_overrun_ms = Some(prev);
                self.fewer_threads += 1;
                log.push(Transition::ThreadsLowered(self.fewer_threads));
            }
            self.last_overrun_ms = Some(snap.now_ms);
        }
        self.last_faults = Some(snap.recorder_faults);

        // Free memory, with a margin: pause under 2 GB, resume above 3 GB.
        if snap.free_ram_bytes < PAUSE_BELOW_BYTES {
            self.low_memory = true;
        } else if snap.free_ram_bytes >= RESUME_ABOVE_BYTES {
            self.low_memory = false;
        }

        let overrun_recent = self
            .last_overrun_ms
            .is_some_and(|t| snap.now_ms.saturating_sub(t) < OVERRUN_CLEAR_MS);

        // The order is the order of importance of the reason shown to the operator.
        let pause = if snap.operator_paused {
            Some(PauseReason::Operator)
        } else if snap.record_only {
            Some(PauseReason::RecordOnly)
        } else if snap.recording && !snap.process_while_recording {
            Some(PauseReason::RecordingRefused)
        } else if snap.live_active && snap.cpu_only {
            Some(PauseReason::LiveOnCpuOnly)
        } else if overrun_recent {
            Some(PauseReason::RecorderOverrun)
        } else if self.low_memory {
            Some(PauseReason::LowMemory)
        } else if snap.on_battery && snap.pause_on_battery {
            Some(PauseReason::OnBattery)
        } else {
            None
        };

        match (self.current, pause) {
            (None, Some(r)) => log.push(Transition::Paused(r)),
            (Some(old), None) => log.push(Transition::Resumed(old)),
            (Some(old), Some(new)) if old != new => {
                log.push(Transition::Resumed(old));
                log.push(Transition::Paused(new));
            }
            _ => {}
        }
        self.current = pause;
        Verdict {
            pause,
            fewer_threads: self.fewer_threads,
        }
    }
}

/// Priority of a job kind: lower runs first. Live (Phase 7) would be 0; import is processed like a rolling job
/// because someone is waiting for it.
pub fn priority(kind: JobKind) -> u8 {
    match kind {
        JobKind::Rolling | JobKind::Import => 1,
        JobKind::Night => 2,
        JobKind::Translate => 3,
    }
}

/// Which job the next free engine works on: the runnable job with the best priority, and within a priority
/// the earliest in queue order (`position`, which Move Up changes). `has_work` says whether the job has a
/// unit that can start now.
pub fn pick_job(jobs: &[Job], has_work: impl Fn(&Job) -> bool) -> Option<&Job> {
    jobs.iter()
        .filter(|j| matches!(j.state, JobState::Queued | JobState::Running) && has_work(j))
        .min_by_key(|j| (priority(j.kind), j.position))
}

/// Why an engine may not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartRefused {
    pub free_after_bytes: i128,
    pub needed_bytes: u64,
}

/// The start gate: free RAM after the engine's measured peak commit (plus the local memory of an integrated
/// graphics adapter it will use, which is system RAM) must stay at least 3 GB. `other_headroom_bytes` is
/// what a second engine already running may still grow by (its measured peak minus what it holds now); the
/// second engine of a hybrid counts it so the two together cannot cross the reserve.
pub fn start_gate(
    free_ram_bytes: u64,
    peak_commit_bytes: u64,
    gpu_local_bytes: u64,
    other_headroom_bytes: u64,
) -> Result<(), StartRefused> {
    let need = peak_commit_bytes
        .saturating_add(gpu_local_bytes)
        .saturating_add(other_headroom_bytes);
    let after = i128::from(free_ram_bytes) - i128::from(need);
    if after >= i128::from(START_RESERVE_BYTES) {
        Ok(())
    } else {
        Err(StartRefused {
            free_after_bytes: after,
            needed_bytes: need.saturating_add(START_RESERVE_BYTES),
        })
    }
}

/// The per-engine memory limit: max(peak commit x 2.0, peak + 1 GiB), capped so the machine keeps 2 GiB free
/// (owner, 14 Nov 2026). `available_ram_bytes` is the machine view's available memory when the engine starts.
pub fn engine_memory_limit(peak_commit_bytes: u64, available_ram_bytes: u64) -> u64 {
    let want = peak_commit_bytes
        .saturating_mul(2)
        .max(peak_commit_bytes.saturating_add(GIB));
    let cap = available_ram_bytes.saturating_sub(PAUSE_BELOW_BYTES);
    want.min(cap.max(peak_commit_bytes))
}

/// The smoke run fails if an engine goes above 70% of its limit.
pub fn over_safe_share(peak_bytes: u64, limit_bytes: u64) -> bool {
    u128::from(peak_bytes) * 10 > u128::from(limit_bytes) * 7
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Job, JobKind, JobState};

    fn snap() -> Snapshot {
        Snapshot {
            now_ms: 0,
            recording: false,
            process_while_recording: true,
            record_only: false,
            free_ram_bytes: 8 * GIB,
            on_battery: false,
            pause_on_battery: true,
            recorder_faults: 0,
            live_active: false,
            cpu_only: true,
            operator_paused: false,
        }
    }

    fn run(s: &mut Scheduler, snap: &Snapshot) -> (Verdict, Vec<Transition>) {
        let mut log = Vec::new();
        let v = s.evaluate(snap, &mut log);
        (v, log)
    }

    fn job(id: i64, kind: JobKind, position: i64, state: JobState) -> Job {
        Job {
            id,
            session: "s".into(),
            kind,
            models_by_lang: serde_json::Value::Null,
            languages: vec![],
            revision: "post-1".into(),
            state,
            cursor_ms: 0,
            position,
            created_ms: 0,
            started_ms: None,
            finished_ms: None,
            error: None,
        }
    }

    #[test]
    fn a_quiet_machine_runs() {
        let mut s = Scheduler::new();
        let (v, log) = run(&mut s, &snap());
        assert_eq!(v.pause, None);
        assert!(log.is_empty());
    }

    #[test]
    fn record_only_and_a_refusing_profile_do_not_process_while_recording() {
        let mut s = Scheduler::new();
        let (v, _) = run(
            &mut s,
            &Snapshot {
                record_only: true,
                ..snap()
            },
        );
        assert_eq!(v.pause, Some(PauseReason::RecordOnly));
        let mut s = Scheduler::new();
        let (v, _) = run(
            &mut s,
            &Snapshot {
                recording: true,
                process_while_recording: false,
                ..snap()
            },
        );
        assert_eq!(v.pause, Some(PauseReason::RecordingRefused));
        // The same profile processes when nothing is recording (after Stop).
        let mut s = Scheduler::new();
        let (v, _) = run(
            &mut s,
            &Snapshot {
                recording: false,
                process_while_recording: false,
                ..snap()
            },
        );
        assert_eq!(v.pause, None);
    }

    #[test]
    fn memory_pauses_under_2_gb_and_resumes_only_above_3_gb() {
        let mut s = Scheduler::new();
        let (v, log) = run(
            &mut s,
            &Snapshot {
                free_ram_bytes: GIB + GIB / 2,
                ..snap()
            },
        );
        assert_eq!(v.pause, Some(PauseReason::LowMemory));
        assert_eq!(log, vec![Transition::Paused(PauseReason::LowMemory)]);
        // 2.5 GB is above the pause line but under the resume line: still paused (no flapping).
        let (v, log) = run(
            &mut s,
            &Snapshot {
                free_ram_bytes: 2 * GIB + GIB / 2,
                ..snap()
            },
        );
        assert_eq!(v.pause, Some(PauseReason::LowMemory));
        assert!(log.is_empty());
        let (v, log) = run(
            &mut s,
            &Snapshot {
                free_ram_bytes: 3 * GIB,
                ..snap()
            },
        );
        assert_eq!(v.pause, None);
        assert_eq!(log, vec![Transition::Resumed(PauseReason::LowMemory)]);
    }

    #[test]
    fn battery_pauses_unless_the_operator_overrode_it() {
        let mut s = Scheduler::new();
        assert_eq!(
            run(
                &mut s,
                &Snapshot {
                    on_battery: true,
                    ..snap()
                }
            )
            .0
            .pause,
            Some(PauseReason::OnBattery)
        );
        let mut s = Scheduler::new();
        assert_eq!(
            run(
                &mut s,
                &Snapshot {
                    on_battery: true,
                    pause_on_battery: false,
                    ..snap()
                }
            )
            .0
            .pause,
            None
        );
    }

    #[test]
    fn an_overrun_pauses_for_a_minute_and_a_second_one_lowers_the_threads() {
        let mut s = Scheduler::new();
        run(
            &mut s,
            &Snapshot {
                now_ms: 0,
                ..snap()
            },
        );
        let (v, log) = run(
            &mut s,
            &Snapshot {
                now_ms: 2_000,
                recorder_faults: 1,
                ..snap()
            },
        );
        assert_eq!(v.pause, Some(PauseReason::RecorderOverrun));
        assert_eq!(log, vec![Transition::Paused(PauseReason::RecorderOverrun)]);
        assert_eq!(v.fewer_threads, 0);
        // Still paused 30 s later; resumed 60 s after the overrun.
        assert!(
            run(
                &mut s,
                &Snapshot {
                    now_ms: 32_000,
                    recorder_faults: 1,
                    ..snap()
                }
            )
            .0
            .pause
            .is_some()
        );
        let (v, log) = run(
            &mut s,
            &Snapshot {
                now_ms: 62_000,
                recorder_faults: 1,
                ..snap()
            },
        );
        assert_eq!(v.pause, None);
        assert_eq!(log, vec![Transition::Resumed(PauseReason::RecorderOverrun)]);
        // A second overrun within 10 minutes: one thread fewer.
        let (v, log) = run(
            &mut s,
            &Snapshot {
                now_ms: 200_000,
                recorder_faults: 2,
                ..snap()
            },
        );
        assert_eq!(v.fewer_threads, 1);
        assert!(log.contains(&Transition::ThreadsLowered(1)));
        // A third one far later does not lower it again.
        let (v, _) = run(
            &mut s,
            &Snapshot {
                now_ms: 5_000_000,
                recorder_faults: 3,
                ..snap()
            },
        );
        assert_eq!(v.fewer_threads, 1);
    }

    #[test]
    fn counters_that_were_already_high_at_the_start_are_not_a_new_overrun() {
        let mut s = Scheduler::new();
        let (v, _) = run(
            &mut s,
            &Snapshot {
                recorder_faults: 7,
                ..snap()
            },
        );
        assert_eq!(v.pause, None);
    }

    #[test]
    fn live_on_a_cpu_only_machine_pauses_the_queue() {
        let mut s = Scheduler::new();
        let (v, _) = run(
            &mut s,
            &Snapshot {
                live_active: true,
                cpu_only: true,
                ..snap()
            },
        );
        assert_eq!(v.pause, Some(PauseReason::LiveOnCpuOnly));
        let mut s = Scheduler::new();
        let (v, _) = run(
            &mut s,
            &Snapshot {
                live_active: true,
                cpu_only: false,
                ..snap()
            },
        );
        assert_eq!(v.pause, None);
    }

    #[test]
    fn the_reason_changes_are_logged_as_a_resume_and_a_pause() {
        let mut s = Scheduler::new();
        run(
            &mut s,
            &Snapshot {
                on_battery: true,
                ..snap()
            },
        );
        let (_, log) = run(
            &mut s,
            &Snapshot {
                operator_paused: true,
                on_battery: true,
                ..snap()
            },
        );
        assert_eq!(
            log,
            vec![
                Transition::Resumed(PauseReason::OnBattery),
                Transition::Paused(PauseReason::Operator)
            ]
        );
    }

    #[test]
    fn rolling_runs_before_night_and_night_before_translation_and_move_up_orders_within_a_kind() {
        let jobs = vec![
            job(1, JobKind::Translate, 1, JobState::Queued),
            job(2, JobKind::Night, 2, JobState::Queued),
            job(3, JobKind::Rolling, 4, JobState::Queued),
            job(4, JobKind::Rolling, 3, JobState::Queued),
            job(5, JobKind::Rolling, 0, JobState::Paused),
        ];
        assert_eq!(pick_job(&jobs, |_| true).map(|j| j.id), Some(4));
        // Rolling with nothing to do yet: night goes next.
        assert_eq!(
            pick_job(&jobs, |j| j.kind != JobKind::Rolling).map(|j| j.id),
            Some(2)
        );
        assert_eq!(pick_job(&jobs, |_| false).map(|j| j.id), None);
    }

    #[test]
    fn the_start_gate_needs_3_gb_after_the_peak() {
        assert!(start_gate(8 * GIB, 4 * GIB, 0, 0).is_ok());
        assert!(
            start_gate(8 * GIB, 5 * GIB, 0, 0).is_ok(),
            "exactly 3 GB left is allowed"
        );
        let e = start_gate(8 * GIB, 5 * GIB + 1, 0, 0).unwrap_err();
        assert!(e.free_after_bytes < i128::from(3 * GIB));
        // An integrated GPU's local memory is system RAM and counts.
        assert!(start_gate(8 * GIB, 4 * GIB, GIB, 0).is_ok());
        assert!(start_gate(8 * GIB, 4 * GIB, 2 * GIB, 0).is_err());
        // The second engine of a hybrid also counts what the first may still grow by.
        assert!(start_gate(8 * GIB, 2 * GIB, 0, 3 * GIB).is_ok());
        assert!(start_gate(8 * GIB, 2 * GIB, 0, 3 * GIB + 1).is_err());
    }

    #[test]
    fn the_engine_memory_limit_is_twice_the_peak_or_a_gib_more_and_keeps_2_gb_free() {
        // Small peak: peak + 1 GiB wins over 2x.
        assert_eq!(engine_memory_limit(GIB / 2, 16 * GIB), GIB / 2 + GIB);
        // Large peak: 2x wins.
        assert_eq!(engine_memory_limit(3 * GIB, 16 * GIB), 6 * GIB);
        // Capped so 2 GiB stay free.
        assert_eq!(engine_memory_limit(3 * GIB, 7 * GIB), 5 * GIB);
        // Never below the peak itself (the start gate already decided it fits).
        assert_eq!(engine_memory_limit(3 * GIB, 4 * GIB), 3 * GIB);
    }

    /// The first spawn smoke (14 Nov 2026) fed the formula 17 MiB of "available" memory by mistake, which gives a
    /// limit equal to the peak (no headroom), and the real engine died at 100% of it. The start gate prevents that
    /// in the app: an engine that may start has at least peak + 3 GiB free, so its limit is at least peak + 1 GiB.
    #[test]
    fn an_engine_the_start_gate_lets_start_always_gets_at_least_a_gib_of_headroom() {
        for peak_mib in [200u64, 800, 1232, 3000, 5000] {
            let peak = peak_mib << 20;
            for extra_mib in [0u64, 1, 512, 4096] {
                let free = peak + 3 * GIB + (extra_mib << 20);
                assert!(start_gate(free, peak, 0, 0).is_ok());
                assert!(
                    engine_memory_limit(peak, free) >= peak + GIB,
                    "peak {peak_mib} MiB free {free}"
                );
            }
        }
        // Without the gate (a bad reading), the limit falls back to the peak itself: no headroom.
        assert_eq!(engine_memory_limit(1232 << 20, 17 << 20), 1232 << 20);
    }

    #[test]
    fn seventy_percent_of_the_limit_is_the_smoke_line() {
        assert!(!over_safe_share(7 * GIB, 10 * GIB));
        assert!(over_safe_share(7 * GIB + 1, 10 * GIB));
    }
}
