//! The queue manager: every open session's queue, the engine slots that work through their units, and the planner
//! that finds the pauses of each closed part and cuts the rolling job into units.
//!
//! Everything an engine process does goes through one [`StartGate`]: the scheduler's pause rules and the 3 GB
//! start reserve. A slot asks it before it starts a unit, the planner asks it before it starts `wb-engine chunks`
//! (owner, 14 Nov 2026: the pause finder is processing, so it obeys the same rules, with the reason logged), and
//! when the gate begins to refuse while an engine is running (free RAM under 2 GB, recording refused, battery,
//! overrun) the manager cancels the running engines: their staged files resume them when the rules allow it.
//! The recorder is never touched: nothing in here shares a lock with it.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use wb_protocol::Event;
use wb_queue::speed::{SpeedLog, UnitSpeed};
use wb_queue::store::{Device, Retry, Segment};
use wb_supervisor::{Cancel, Notice, Observer, RunReport};

use crate::chunks::{ChunksError, ChunksJob, PeakTracker, pauses_for_part};
use crate::exec::{UnitEnd, UnitExecutor};
use crate::gate::{GateEvent, StartGate, StartRequest, wait_for_gate};
use crate::session_queue::{Claim, SessionQueue, read_staged};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the manager tells the app (a Toast, the Queue screen, the live transcript, the scheduler log).
pub trait Events: Send + Sync {
    /// Segments are committed (written, fsynced and in SQLite): safe to show.
    fn committed(&self, session: &str, segments: &[Segment]);
    /// A job, unit or state changed: the Queue screen refreshes.
    fn changed(&self);
    /// A line for the scheduler log (a wait and its reason, a failure, a lowered thread count).
    fn log(&self, level: LogLevel, text: &str);
    /// A job is parked or a failure needs the operator's eyes: a Toast.
    fn problem(&self, text: &str);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// An engine slot: one engine process at a time.
#[derive(Debug, Clone)]
pub struct Slot {
    pub name: String,
    pub device: Device,
}

/// One session the manager works on.
pub struct SessionHandle {
    pub name: String,
    pub queue: Mutex<SessionQueue>,
    /// Where `wb-engine chunks` writes its output for this session's parts.
    pub work_dir: PathBuf,
}

enum PlanRequest {
    Part {
        session: Arc<SessionHandle>,
        job: i64,
        part: u32,
        start_ms: u64,
        end_ms: u64,
        sample_rate: u32,
        wav: PathBuf,
    },
    Finish {
        session: Arc<SessionHandle>,
        job: i64,
    },
}

pub struct Config {
    pub chunks_exe: PathBuf,
    pub models_dir: PathBuf,
    pub scrub_env: bool,
    pub chunks_timeout: Duration,
    /// How often a waiting engine asks the gate again.
    pub gate_poll: Duration,
    pub clock_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub ist_text: Arc<dyn Fn() -> String + Send + Sync>,
}

pub struct Manager {
    sessions: Mutex<Vec<Arc<SessionHandle>>>,
    wake: Condvar,
    wake_lock: Mutex<()>,
    stop: AtomicBool,
    running: Mutex<Vec<(String, Cancel)>>,
    gate: Arc<dyn StartGate>,
    executor: Arc<dyn UnitExecutor>,
    events: Arc<dyn Events>,
    cfg: Config,
    peaks: PeakTracker,
    plan_tx: Mutex<Option<Sender<PlanRequest>>>,
    /// Slots turned off for this run after a fatal failure of their device.
    disabled: Mutex<Vec<Device>>,
    other_device: Mutex<bool>,
    /// What each running engine may still grow by (its measured peak minus what it has reached): the engine that
    /// starts next must leave room for it (AGENTS.md memory rules for two engines at once).
    headroom: Mutex<BTreeMap<String, u64>>,
    /// How long this computer's finished units took against what the profile promised for them.
    speed: Mutex<SpeedLog>,
}

impl Manager {
    pub fn new(
        gate: Arc<dyn StartGate>,
        executor: Arc<dyn UnitExecutor>,
        events: Arc<dyn Events>,
        cfg: Config,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            wake_lock: Mutex::new(()),
            stop: AtomicBool::new(false),
            running: Mutex::new(Vec::new()),
            gate,
            executor,
            events,
            cfg,
            peaks: PeakTracker::default(),
            plan_tx: Mutex::new(None),
            disabled: Mutex::new(Vec::new()),
            other_device: Mutex::new(false),
            headroom: Mutex::new(BTreeMap::new()),
            speed: Mutex::new(SpeedLog::default()),
        })
    }

    /// Measured over promised: how much slower (above 1) or faster (below 1) this computer's finished units ran
    /// than the profile said (the median of the last few). `None` until a unit has finished from its start.
    pub fn speed_factor(&self) -> Option<f64> {
        lock(&self.speed).factor()
    }

    /// How many finished units the speed factor rests on.
    pub fn speed_units(&self) -> usize {
        lock(&self.speed).units()
    }

    /// Adds a finished unit to the speed log. Only a unit that ran from its start in one engine process says
    /// what the profile's number means: a resumed unit covers less audio than its range, and a restarted one
    /// includes the wait for the restart.
    fn note_speed(&self, claim: &Claim, report: &RunReport, took: Duration, fresh: bool) {
        if !fresh || report.restarts > 0 {
            return;
        }
        let mut mix: BTreeMap<String, u64> = BTreeMap::new();
        if let Ok(events) = read_staged(&claim.staged) {
            for e in events {
                *mix.entry(e.lang).or_default() += e.end_ms.saturating_sub(e.start_ms);
            }
        }
        let Some(expected) = self.executor.expected_rtf(claim, &mix) else {
            return;
        };
        lock(&self.speed).record(UnitSpeed {
            audio_ms: claim.unit.end_ms.saturating_sub(claim.unit.start_ms),
            took_s: took.as_secs_f64(),
            expected_rtf: expected,
        });
    }

    /// Whether a second device (a graphics engine beside the processor one) is in use: failures that exhaust one
    /// device then move to the other.
    pub fn set_other_device_available(&self, yes: bool) {
        *lock(&self.other_device) = yes;
    }

    pub fn add_session(&self, handle: Arc<SessionHandle>) {
        lock(&self.sessions).push(handle);
        self.poke();
    }

    pub fn sessions(&self) -> Vec<Arc<SessionHandle>> {
        lock(&self.sessions).clone()
    }

    /// Wakes the slots (new work, a resumed job).
    pub fn poke(&self) {
        let _g = lock(&self.wake_lock);
        self.wake.notify_all();
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.cancel_running("shutting down");
        self.poke();
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Cancels every running engine (the gate began to refuse): each unit is pending again and resumes from its
    /// staged file.
    pub fn cancel_running(&self, why: &str) {
        let running = lock(&self.running);
        if !running.is_empty() {
            self.events.log(
                LogLevel::Info,
                &format!("stopping {} engine(s): {why}", running.len()),
            );
        }
        for (_, c) in running.iter() {
            c.cancel();
        }
    }

    fn sleep_a_little(&self, max: Duration) {
        let g = lock(&self.wake_lock);
        let _ = self
            .wake
            .wait_timeout(g, max)
            .unwrap_or_else(PoisonError::into_inner);
    }

    // ---- planner ----

    /// Starts the planner thread: it finds the pauses of each closed part and cuts the rolling job into units.
    pub fn start_planner(self: &Arc<Self>) -> std::io::Result<std::thread::JoinHandle<()>> {
        let (tx, rx) = channel();
        *lock(&self.plan_tx) = Some(tx);
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("wb-planner".into())
            .spawn(move || me.planner_loop(&rx))
    }

    /// A part closed. `wav` is the part's file.
    #[allow(clippy::too_many_arguments)]
    pub fn part_closed(
        &self,
        session: &Arc<SessionHandle>,
        job: i64,
        part: u32,
        start_ms: u64,
        end_ms: u64,
        sample_rate: u32,
        wav: PathBuf,
    ) {
        self.send_plan(PlanRequest::Part {
            session: Arc::clone(session),
            job,
            part,
            start_ms,
            end_ms,
            sample_rate,
            wav,
        });
    }

    /// Stop was pressed: the last unit ends at the end of the session.
    pub fn session_finished(&self, session: &Arc<SessionHandle>, job: i64) {
        self.send_plan(PlanRequest::Finish {
            session: Arc::clone(session),
            job,
        });
    }

    fn send_plan(&self, request: PlanRequest) {
        if let Some(tx) = lock(&self.plan_tx).as_ref()
            && tx.send(request).is_err()
        {
            self.events
                .log(LogLevel::Error, "the planner is not running");
        }
    }

    fn planner_loop(&self, rx: &Receiver<PlanRequest>) {
        // A request the pause rules interrupted goes back to the front and is tried again when they allow it.
        let mut queue: VecDeque<PlanRequest> = VecDeque::new();
        loop {
            if self.stopping() {
                return;
            }
            let request = match queue.pop_front() {
                Some(r) => r,
                None => match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(r) => r,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                },
            };
            if let Some(again) = self.plan_one(request) {
                queue.push_front(again);
            }
        }
    }

    /// Handles one planning request. Returns it when it was interrupted and must be tried again.
    fn plan_one(&self, request: PlanRequest) -> Option<PlanRequest> {
        match request {
            PlanRequest::Finish { session, job } => {
                let result = lock(&session.queue).plan_units(job, true);
                match result {
                    Ok(n) => {
                        self.events.log(
                            LogLevel::Info,
                            &format!(
                                "{}: session closed, {n} final unit(s) planned",
                                session.name
                            ),
                        );
                    }
                    Err(e) => self.events.problem(&format!("{}: {e}", session.name)),
                }
                self.events.changed();
                self.poke();
                None
            }
            PlanRequest::Part {
                ref session,
                job,
                part,
                start_ms,
                end_ms,
                sample_rate,
                ref wav,
            } => {
                if lock(&session.queue).known_parts().contains(&part) {
                    // Found before a restart: only the units are missing.
                    let _ = lock(&session.queue).plan_units(job, false);
                    self.poke();
                    return None;
                }
                let cancel = Cancel::new();
                lock(&self.running).push((format!("chunks part {part}"), cancel.clone()));
                let job_spec = ChunksJob {
                    exe: &self.cfg.chunks_exe,
                    wav,
                    models_dir: &self.cfg.models_dir,
                    work_dir: &session.work_dir,
                    part,
                    part_start_ms: start_ms,
                    timeout: self.cfg.chunks_timeout,
                    scrub_env: self.cfg.scrub_env,
                    min_pause_ms: 300,
                };
                let name = session.name.clone();
                let mut on_gate = |e: GateEvent| self.log_gate(&name, &e);
                let outcome = pauses_for_part(
                    &job_spec,
                    self.gate.as_ref(),
                    &self.peaks,
                    &cancel,
                    &mut on_gate,
                );
                lock(&self.running).retain(|(n, _)| n != &format!("chunks part {part}"));
                match outcome {
                    Ok((pauses, stats)) => {
                        self.events.log(
                            LogLevel::Info,
                            &format!(
                                "{}: pauses of part {part} found ({} pauses, {} fallback cuts) in {:.1} s; engine peak {} against limit {} ({} peak)",
                                session.name,
                                pauses.pauses.len(),
                                pauses.fallback.len(),
                                stats.seconds,
                                fmt_bytes(stats.peak_memory_bytes),
                                fmt_bytes(stats.memory_limit_bytes),
                                if stats.peak_was_measured { "measured" } else { "assumed" },
                            ),
                        );
                        let mut q = lock(&session.queue);
                        let added = q
                            .add_part(part, start_ms, end_ms, sample_rate, &pauses)
                            .and_then(|()| q.plan_units(job, false));
                        drop(q);
                        match added {
                            Ok(n) if n > 0 => {
                                self.events.log(
                                    LogLevel::Info,
                                    &format!(
                                        "{}: {n} unit(s) planned after part {part}",
                                        session.name
                                    ),
                                );
                            }
                            Ok(_) => {}
                            Err(e) => self.events.problem(&format!("{}: {e}", session.name)),
                        }
                        self.events.changed();
                        self.poke();
                        None
                    }
                    Err(ChunksError::Cancelled) => {
                        // A pause rule stopped it (or shutdown): try again when the rules allow.
                        if self.stopping() {
                            None
                        } else {
                            self.sleep_a_little(Duration::from_secs(2));
                            Some(request)
                        }
                    }
                    Err(e) => {
                        let text = format!(
                            "{}: could not find the pauses of part {part}: {e}. The job is parked; a cut is never guessed.",
                            session.name
                        );
                        let now = (self.cfg.clock_ms)();
                        let _ = lock(&session.queue).park_job(job, &text, now);
                        self.events.problem(&text);
                        self.events.changed();
                        None
                    }
                }
            }
        }
    }

    fn log_gate(&self, who: &str, e: &GateEvent) {
        match e {
            GateEvent::Waiting { what, reason } => self
                .events
                .log(LogLevel::Info, &format!("{who}: {what} waits: {reason}")),
            GateEvent::Cleared { what, waited_ms } => self.events.log(
                LogLevel::Info,
                &format!(
                    "{who}: {what} starts after waiting {:.1} s",
                    *waited_ms as f64 / 1000.0
                ),
            ),
        }
    }

    // ---- slots ----

    /// Starts one worker thread for `slot`.
    pub fn start_slot(
        self: &Arc<Self>,
        slot: Slot,
    ) -> std::io::Result<std::thread::JoinHandle<()>> {
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name(format!("wb-slot-{}", slot.name))
            .spawn(move || me.slot_loop(&slot))
    }

    /// What the other running engines may still grow by, for the engine of `slot` that is about to start.
    fn other_headroom(&self, slot: &str) -> u64 {
        lock(&self.headroom)
            .iter()
            .filter(|(n, _)| n.as_str() != slot)
            .map(|(_, v)| *v)
            .sum()
    }

    fn claim_any(&self, device: Device) -> Option<(Arc<SessionHandle>, Claim)> {
        let sessions = self.sessions();
        let mut best: Option<(usize, (u8, i64))> = None;
        for (i, s) in sessions.iter().enumerate() {
            let Ok(Some(p)) = lock(&s.queue).peek(device) else {
                continue;
            };
            if best.is_none_or(|(_, bp)| p < bp) {
                best = Some((i, p));
            }
        }
        let (i, _) = best?;
        let session = sessions.get(i)?.clone();
        let now = (self.cfg.clock_ms)();
        let claim = lock(&session.queue).claim(device, now).ok().flatten()?;
        Some((session, claim))
    }

    fn slot_loop(&self, slot: &Slot) {
        let mut last_pause: Option<String> = None;
        while !self.stopping() {
            if lock(&self.disabled).contains(&slot.device) {
                self.sleep_a_little(Duration::from_secs(5));
                continue;
            }
            // The scheduler's pause rules first, so no unit is claimed (and shown as running) while they hold.
            let paused = self.gate.paused();
            if paused != last_pause {
                match &paused {
                    Some(reason) => self.events.log(
                        LogLevel::Info,
                        &format!("{}: waiting, not processing: {reason}", slot.name),
                    ),
                    None => self.events.log(
                        LogLevel::Info,
                        &format!("{}: processing may continue", slot.name),
                    ),
                }
                last_pause = paused.clone();
            }
            if paused.is_some() {
                self.sleep_a_little(self.cfg.gate_poll);
                continue;
            }
            let Some((session, claim)) = self.claim_any(slot.device) else {
                self.sleep_a_little(Duration::from_millis(1000));
                continue;
            };
            let what = format!(
                "unit {} of {} job {} on {}",
                claim.unit.idx, claim.job.revision, claim.job.id, slot.name
            );
            let cancel = Cancel::new();
            lock(&self.running).push((slot.name.clone(), cancel.clone()));
            let (peak, gpu_local) = self.executor.expected_memory(&claim);
            let request = StartRequest {
                what: what.clone(),
                peak_commit_bytes: peak,
                gpu_local_bytes: gpu_local,
                other_headroom_bytes: self.other_headroom(&slot.name),
            };
            let name = session.name.clone();
            let mut on_gate = |e: GateEvent| self.log_gate(&name, &e);
            let grant = wait_for_gate(
                self.gate.as_ref(),
                &request,
                &cancel,
                self.cfg.gate_poll,
                &mut on_gate,
            );
            let Some(grant) = grant else {
                lock(&self.running).retain(|(n, _)| n != &slot.name);
                let _ = lock(&session.queue).release(&claim);
                continue;
            };
            // Until its own peak is read, a running engine may still grow by all of its expected peak.
            lock(&self.headroom).insert(slot.name.clone(), peak.saturating_add(gpu_local));
            self.events.changed();
            // A unit with nothing staged runs from its start: its time says something about speed.
            let fresh = read_staged(&claim.staged).is_ok_and(|s| s.is_empty());
            let started = Instant::now();
            let mut obs = SlotObserver {
                events: self.events.as_ref(),
                who: format!("{}: {what}", session.name),
                headroom: &self.headroom,
                slot: slot.name.clone(),
                expected_peak: peak.saturating_add(gpu_local),
            };
            let end = self.executor.run(&claim, grant, &cancel, &mut obs);
            lock(&self.headroom).remove(&slot.name);
            lock(&self.running).retain(|(n, _)| n != &slot.name);
            let timing = UnitTiming {
                took: started.elapsed(),
                fresh,
            };
            self.finish_unit(&session, &claim, slot, end, timing, grant);
            self.events.changed();
        }
    }

    fn finish_unit(
        &self,
        session: &Arc<SessionHandle>,
        claim: &Claim,
        slot: &Slot,
        end: UnitEnd,
        timing: UnitTiming,
        grant: crate::gate::Grant,
    ) {
        let now = (self.cfg.clock_ms)();
        let UnitTiming { took, fresh } = timing;
        match end {
            UnitEnd::Done(report) => {
                self.note_speed(claim, &report, took, fresh);
                self.events.log(
                    LogLevel::Info,
                    &format!(
                        "{}: unit {} of {} done in {:.1} s on {} ({} restart(s), engine peak {} against limit {}{})",
                        session.name,
                        claim.unit.idx,
                        claim.job.revision,
                        took.as_secs_f64(),
                        slot.name,
                        report.restarts,
                        fmt_bytes(report.peak_memory_bytes),
                        fmt_bytes(grant.memory_limit_bytes),
                        if report.memory_limit_deaths > 0 {
                            format!(", {} memory-limit death(s)", report.memory_limit_deaths)
                        } else {
                            String::new()
                        }
                    ),
                );
                let ist = (self.cfg.ist_text)();
                let completed = lock(&session.queue).complete(claim, &ist, now);
                match completed {
                    Ok(c) => {
                        if c.refused > 0 {
                            self.events.log(
                                LogLevel::Warn,
                                &format!(
                                    "{}: {} segment(s) of unit {} were refused as repeats",
                                    session.name, c.refused, claim.unit.idx
                                ),
                            );
                        }
                        if !c.segments.is_empty() {
                            self.events.committed(&session.name, &c.segments);
                        }
                        if c.job_done {
                            self.events.log(
                                LogLevel::Info,
                                &format!(
                                    "{}: {} transcript complete",
                                    session.name, claim.job.revision
                                ),
                            );
                        }
                    }
                    Err(e) => {
                        self.events.problem(&format!(
                            "{}: could not commit unit {}: {e}",
                            session.name, claim.unit.idx
                        ));
                    }
                }
            }
            UnitEnd::Cancelled => {
                let _ = lock(&session.queue).release(claim);
            }
            UnitEnd::Failed(error) => {
                let other = *lock(&self.other_device);
                let retry = lock(&session.queue).fail(claim, &error, other, now);
                match retry {
                    Ok(Retry::Same(d)) => self.events.log(
                        LogLevel::Warn,
                        &format!(
                            "{}: unit {} failed on the {}: {error}; trying again there",
                            session.name,
                            claim.unit.idx,
                            d.as_str()
                        ),
                    ),
                    Ok(Retry::Other(d)) => self.events.log(
                        LogLevel::Warn,
                        &format!(
                            "{}: unit {} failed three times on the {}: {error}; moving it to the {}",
                            session.name,
                            claim.unit.idx,
                            claim.device.as_str(),
                            d.as_str()
                        ),
                    ),
                    Ok(Retry::GiveUp) => self.events.problem(&format!(
                        "{}: unit {} failed on every device: {error}. The job is parked.",
                        session.name, claim.unit.idx
                    )),
                    Err(e) => self
                        .events
                        .problem(&format!("{}: {e}", session.name)),
                }
            }
        }
    }
}

/// How long a unit's engine run took and whether it ran from the unit's start (nothing was staged before).
#[derive(Clone, Copy)]
struct UnitTiming {
    took: Duration,
    fresh: bool,
}

fn fmt_bytes(b: Option<u64>) -> String {
    match b {
        Some(v) => format!("{:.0} MiB", v as f64 / (1u64 << 20) as f64),
        None => "unknown".into(),
    }
}

struct SlotObserver<'a> {
    events: &'a dyn Events,
    who: String,
    headroom: &'a Mutex<BTreeMap<String, u64>>,
    slot: String,
    expected_peak: u64,
}

impl Observer for SlotObserver<'_> {
    fn event(&mut self, event: &Event) {
        // The engine says why it failed in an `error` event just before it exits; without this the log would
        // show only "Died { exit: 1 }" (found by the day-simulation smoke).
        match event {
            Event::Error {
                code,
                message,
                fatal,
            } => self.events.log(
                LogLevel::Warn,
                &format!(
                    "{}: engine error {code}{}: {message}",
                    self.who,
                    if *fatal { " (fatal)" } else { "" }
                ),
            ),
            Event::Warning { code, message } => self.events.log(
                LogLevel::Info,
                &format!("{}: engine warning {code}: {message}", self.who),
            ),
            _ => {}
        }
    }

    fn notice(&mut self, notice: &Notice) {
        let level = match notice {
            Notice::MemoryLimit { .. } => LogLevel::Warn,
            _ => LogLevel::Info,
        };
        self.events.log(level, &format!("{}: {notice:?}", self.who));
    }

    fn memory(&mut self, peak_bytes: u64) {
        lock(self.headroom).insert(
            self.slot.clone(),
            self.expected_peak.saturating_sub(peak_bytes),
        );
    }
}

#[cfg(test)]
mod observer_tests {
    use super::*;

    #[derive(Default)]
    struct Lines(Mutex<Vec<(LogLevel, String)>>);

    impl Events for Lines {
        fn committed(&self, _session: &str, _segments: &[Segment]) {}
        fn changed(&self) {}
        fn log(&self, level: LogLevel, text: &str) {
            lock(&self.0).push((level, text.to_owned()));
        }
        fn problem(&self, _text: &str) {}
    }

    #[test]
    fn an_engines_error_event_reaches_the_scheduler_log() {
        let lines = Lines::default();
        let headroom = Mutex::new(BTreeMap::new());
        let mut obs = SlotObserver {
            events: &lines,
            who: "s: unit 0".into(),
            headroom: &headroom,
            slot: "gpu".into(),
            expected_peak: 0,
        };
        obs.event(&Event::Error {
            code: "out_of_memory".into(),
            message: "the graphics driver refused 1 GiB".into(),
            fatal: true,
        });
        obs.event(&Event::Heartbeat {
            unit: 0,
            phase: "compute".into(),
        });
        let got = lock(&lines.0).clone();
        assert_eq!(got.len(), 1, "only the error is logged: {got:?}");
        assert_eq!(got[0].0, LogLevel::Warn);
        assert!(got[0].1.contains("out_of_memory"));
        assert!(got[0].1.contains("refused 1 GiB"));
    }
}
