//! The post-processing queue in the app (Phase 4): the jobs of every open session, the engine slots that work
//! through their units, the scheduler's pause rules and the commands the Queue screen uses.
//!
//! The coordination logic is in `crates/wb-runner` and `crates/wb-queue` (no Tauri, tested with fake engines);
//! this module supplies what only the app knows: the machine's readings, the capability profile in use, the
//! recorder's closed parts and fault counters (through `feed`, which can never block the recorder) and the UI
//! events. Nothing here touches the recorder: it reads `parts.json` and the files the recorder wrote.

pub mod core;
#[cfg(any(test, feature = "gate-hooks"))]
pub mod day_stop;
pub mod executor;
pub mod feed;
#[cfg(feature = "gate-hooks")]
pub mod gate_hooks;
pub mod langlock;
pub mod night_route;
pub mod profile;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use wb_capability::DeviceConfig;
use wb_queue::store::{Device, JobKind, JobState, Segment, UnitState};
use wb_recorder::events::{Notice, NoticeLevel};
use wb_recorder::parts::PartRecord;
use wb_runner::manager::{Config, Events, LogLevel, Manager, SessionHandle, Slot};
use wb_runner::session_queue::SessionQueue;

use self::core::{AppGate, Core, Readings};
use self::executor::{AppExecutor, Environment, SessionInfoForUnits};
use self::feed::Feed;
use self::profile::ProfileView;
use crate::boundary::blocking;
use crate::state::{AppState, lock};

fn plock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The time each step of a start took, for the log. Starting a session must never wait for the queue; when it does,
/// these numbers say where (20 Nov 2026: in the first full day simulation the player of the rig began 1.8 to 46 s after
/// the recorder, and nothing said whether the app or the script had been slow).
pub struct Laps {
    began: Instant,
    t: Instant,
    parts: Vec<String>,
}

impl Laps {
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            began: now,
            t: now,
            parts: Vec::new(),
        }
    }

    /// Closes the step that began at the last lap (or at the start).
    pub fn lap(&mut self, label: &str) {
        self.parts
            .push(format!("{label} {} ms", self.t.elapsed().as_millis()));
        self.t = Instant::now();
    }

    /// The steps and the total, as `a 3 ms, b 12 ms, total 15 ms`.
    pub fn text(&self) -> String {
        format!(
            "{}, total {} ms",
            self.parts.join(", "),
            self.began.elapsed().as_millis()
        )
    }
}

/// One line of start timings: the application log and, in a gate build, the scheduler log file of the run.
pub fn log_start_timing(line: &str) {
    tracing::info!("{line}");
    #[cfg(feature = "gate-hooks")]
    gate_hooks::queue_log(&format!("START: {line}"));
}

/// How often the monitor re-reads the machine and `parts.json`.
const MONITOR_STEP: Duration = Duration::from_secs(2);
const PARTS_RECHECK: Duration = Duration::from_secs(5);
const NIGHT_CHECK: Duration = Duration::from_secs(30);
/// The STELMAR 2027 programme, built into the app (the night pass waits for the last session of the day).
const PROGRAMME_JSON: &str = include_str!("../../../docs/programme.json");

// ------------------------------------------------------------------------------------------------ the clock

/// Milliseconds since the Unix epoch. In a gate build `WB_GATE_CLOCK_OFFSET_S` moves it so a short test can
/// stand for the day's programme (10:00 IST); every other build reads the system clock and nothing else.
/// Gate builds may shorten the night pass's 30-minute wait (`WB_GATE_NIGHT_WAIT_S`); every other build waits 30
/// minutes.
fn night_wait_shift_ms() -> u64 {
    #[cfg(feature = "gate-hooks")]
    {
        gate_hooks::night_wait_shift_ms()
    }
    #[cfg(not(feature = "gate-hooks"))]
    {
        0
    }
}

pub fn clock_ms() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    now.saturating_add_signed(clock_offset_ms())
}

/// The gate build's clock offset (`WB_GATE_CLOCK_OFFSET_S`); zero in every other build.
fn clock_offset_ms() -> i64 {
    #[cfg(feature = "gate-hooks")]
    {
        std::env::var("WB_GATE_CLOCK_OFFSET_S")
            .ok()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0)
            .saturating_mul(1000)
    }
    #[cfg(not(feature = "gate-hooks"))]
    {
        0
    }
}

/// The profile in use as the queue reads it. A gate build can force a hybrid configuration for the kill test
/// (`WB_GATE_FORCE_HYBRID`), which its report labels.
fn view_of(cap: &wb_capability::Capability, changes: Vec<String>) -> ProfileView {
    #[allow(unused_mut)]
    let mut view = ProfileView::from_capability(cap, changes);
    #[cfg(feature = "gate-hooks")]
    if gate_hooks::force_hybrid() && !view.record_only {
        let gpu = view.gpu_index.unwrap_or(0);
        view.config = Some(DeviceConfig::Hybrid { gpu });
        view.gpu_index = Some(gpu);
    }
    view
}

fn ist_text() -> String {
    wb_recorder::util::rfc3339(wb_recorder::util::now_ist())
}

// -------------------------------------------------------------------------------------------- readings

/// The scheduler's inputs on a real computer.
struct AppReadings {
    app: AppHandle,
    feed: Arc<Feed>,
    free: Mutex<Option<(Instant, u64)>>,
    /// The slowest and the number of profile reads (the profile is read at every scheduling decision, not cached).
    profile_reads: std::sync::atomic::AtomicU64,
    profile_read_max_us: std::sync::atomic::AtomicU64,
    /// The sum of all reads, for the mean (microseconds).
    profile_read_total_us: std::sync::atomic::AtomicU64,
}

impl Readings for AppReadings {
    fn now_ms(&self) -> u64 {
        clock_ms()
    }

    fn free_ram_bytes(&self) -> u64 {
        // One probe a second is plenty and keeps the gate's polling cheap. This is the probe's available RAM, nothing
        // subtracted: the host's free memory, or the job's remaining limit under emulation. The engines' graphics memory is
        // NOT taken off here. The start gate takes the measured graphics memory of the engine it is about to start from
        // `memory_for_models` (its second value) beside the engine's peak; memory an engine already holds is already out of the
        // available RAM the probe reports (N4 of the auditor of 20 Nov 2026: the comment used to say otherwise).
        let mut cached = plock(&self.free);
        if let Some((at, v)) = *cached
            && at.elapsed() < Duration::from_secs(1)
        {
            return v;
        }
        let v = wb_machine::probe().available_ram_bytes;
        *cached = Some((Instant::now(), v));
        v
    }

    fn recording(&self) -> bool {
        self.app
            .state::<AppState>()
            .recording
            .load(Ordering::SeqCst)
    }

    fn on_battery(&self) -> bool {
        crate::syscheck::on_battery()
    }

    fn recorder_faults(&self) -> u64 {
        self.feed.recorder_faults()
    }

    fn profile(&self) -> Option<ProfileView> {
        // Read fresh every time: a new check, a fingerprint change or a changed refusal rule must show in the
        // very next scheduling decision. The cost is measured and goes into the report.
        let t = Instant::now();
        let v =
            crate::syscheck::profile_in_use(&self.app).map(|(cap, changes)| view_of(&cap, changes));
        let us = u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.profile_reads.fetch_add(1, Ordering::Relaxed);
        self.profile_read_max_us.fetch_max(us, Ordering::Relaxed);
        self.profile_read_total_us.fetch_add(us, Ordering::Relaxed);
        v
    }
}

// ----------------------------------------------------------------------------------------------- events

struct AppEvents {
    app: AppHandle,
    core: Arc<Core>,
}

impl Events for AppEvents {
    fn committed(&self, session: &str, segments: &[Segment]) {
        self.app.state::<AppState>().broadcast(serde_json::json!({
            "kind": "segment_committed",
            "session": session,
            "segments": segments,
        }));
    }

    fn changed(&self) {
        self.app
            .state::<AppState>()
            .broadcast(serde_json::json!({ "kind": "queue_update" }));
    }

    fn log(&self, level: LogLevel, text: &str) {
        match level {
            LogLevel::Info => tracing::info!(target: "queue", "{text}"),
            LogLevel::Warn => tracing::warn!(target: "queue", "{text}"),
            LogLevel::Error => tracing::error!(target: "queue", "{text}"),
        }
        self.core.push_log(text.to_owned());
    }

    fn problem(&self, text: &str) {
        tracing::error!(target: "queue", "{text}");
        self.core.push_log(text.to_owned());
        let _ = self.app.state::<AppState>().events.send(Notice::event(
            NoticeLevel::Error,
            "queue",
            text.to_owned(),
        ));
    }

    fn without_hinglish(&self, session: &str, _job: i64, _unit: u32) {
        if let Some(q) = self.app.try_state::<QueueState>() {
            q.0.note_without_hinglish(session);
        }
    }
}

// ------------------------------------------------------------------------------------------------- state

struct OpenSession {
    name: String,
    dir: PathBuf,
    handle: Arc<SessionHandle>,
    job: i64,
    languages: Vec<String>,
    /// Parts already handed to the planner.
    submitted: BTreeSet<u32>,
    /// Stop was seen and the final cut was asked for.
    finished: bool,
    /// When Stop was seen (the scheduler's clock): the night pass starts 30 minutes after the last one.
    ended_ms: Option<u64>,
    /// The night pass has been planned for this session (or skipped, with the reason logged).
    night_handled: bool,
    /// The night pass planned for this session was cancelled because another session started (its job is held,
    /// with the units it committed); it goes on when the pass is planned again after the last session.
    night_cancelled: bool,
    /// Units of this session were decoded on the processor without the Hinglish model after a graphics failure (the
    /// session's `fallback-units.json`): the night pass redoes it with the Hinglish model, or it is marked for a faster
    /// machine (owner, 27 Nov 2026).
    fell_back: bool,
}

pub struct Queue {
    app: AppHandle,
    core: Arc<Core>,
    manager: Arc<Manager>,
    executor: Arc<AppExecutor>,
    open: Arc<Mutex<Vec<OpenSession>>>,
    slots_started: AtomicBool,
    /// The night safety switch has said once that no Hinglish night job is left to send back (it says it once).
    night_switch_said: AtomicBool,
    feed: Arc<Feed>,
    /// Read by the gate build's report (how often and how slowly the profile was read).
    #[cfg(feature = "gate-hooks")]
    readings: Arc<AppReadings>,
}

/// What the commands reach through Tauri's managed state.
pub struct QueueState(pub Arc<Queue>);

/// The folder of the session that was stopped, from what the recorder's snapshot says. Stop clears the snapshot's
/// `session` (found by the day-simulation smoke: reading only that field never saw a Stop), so the folder comes from
/// `last_stop`, which stays until the next Stop. It is not a stop while the same folder is recording again (a session
/// resumed after recovery).
pub fn stopped_dir(
    recording: bool,
    current: Option<&str>,
    last_stop: Option<&str>,
) -> Option<String> {
    let dir = last_stop?;
    if recording && current == Some(dir) {
        return None;
    }
    Some(dir.to_owned())
}

/// Whether the open session in `dir` has stopped. Two signals say so: the recorder's `last_stop` names its folder, or
/// another session is recording now (the machine has one recorder, so every other session has ended). The second one
/// exists because Start rebuilds the recorder's snapshot and clears `last_stop`: a session started within one poll of
/// the monitor (5 s) after a Stop hid that Stop for good, so the first session was never finalised and the night pass
/// waited for it for ever (day simulation of 20 Nov 2026: Parallel II, 3.3 s before Panel).
pub fn session_has_stopped(
    stopped: Option<&str>,
    recording_dir: Option<&str>,
    dir: &std::path::Path,
) -> bool {
    stopped.is_some_and(|d| std::path::Path::new(d) == dir)
        || recording_dir.is_some_and(|d| std::path::Path::new(d) != dir)
}

/// The languages a job decodes: English, and at most one of Hindi (Devanagari `hi`) or Urdu (Nastaliq `ur`). A
/// list that names both gets Hindi (owner, 24 Nov 2026); [`language_warning`] says so. The rule is
/// `wb_queue::languages::session_languages`.
pub fn job_languages(expected: &[String], script: Option<&str>) -> Vec<String> {
    wb_queue::languages::session_languages("", expected, script).languages
}

/// Writes the one-language warning (naming the session) to the Queue log. Never blocks anything.
fn log_language_warning(
    queue: &Queue,
    dir: &std::path::Path,
    expected: &[String],
    script: Option<&str>,
) {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("this session");
    if let Some(w) = language_warning(name, expected, script) {
        queue.core.push_log(w);
    }
}

/// What to tell the organiser about a session whose languages broke the one-language rule, naming the session.
pub fn language_warning(
    session: &str,
    expected: &[String],
    script: Option<&str>,
) -> Option<String> {
    wb_queue::languages::session_languages(session, expected, script).warning
}

fn models_json(models: &std::collections::BTreeMap<String, String>) -> serde_json::Value {
    serde_json::Value::Object(
        models
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect(),
    )
}

impl Queue {
    /// Starts the engine slots the profile in use calls for (once): a processor slot, a graphics slot, or both
    /// for a hybrid.
    fn ensure_slots(&self, profile: &ProfileView) {
        if self.slots_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(config) = profile.config else {
            self.slots_started.store(false, Ordering::SeqCst);
            return;
        };
        let (cpu, gpu) = match config {
            DeviceConfig::Cpu => (true, false),
            DeviceConfig::Gpu { .. } => (false, true),
            DeviceConfig::Hybrid { .. } => (true, true),
        };
        self.manager.set_other_device_available(cpu && gpu);
        for (wanted, name, device) in [(cpu, "cpu", Device::Cpu), (gpu, "gpu", Device::Gpu)] {
            if !wanted {
                continue;
            }
            let slot = Slot {
                name: name.into(),
                device,
            };
            if let Err(e) = self.manager.start_slot(slot) {
                tracing::error!("could not start the {name} engine slot: {e}");
            } else {
                self.core.push_log(format!(
                    "{name} engine slot started (configuration {config:?}, {} thread(s))",
                    profile.cpu_threads
                ));
            }
        }
    }

    /// Opens (or reopens) a session's queue and registers it. `languages` are the job's decoding languages.
    fn register(
        &self,
        dir: PathBuf,
        languages: &[String],
        profile: &ProfileView,
    ) -> Result<(), String> {
        let mut laps = Laps::start();
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session")
            .to_owned();
        if plock(&self.open).iter().any(|s| s.name == name) {
            return Ok(());
        }
        laps.lap("open-sessions lock");
        let models = profile.rolling_models(languages);
        if models.is_empty() {
            return Err(format!(
                "no transcription model was measured for the languages of {name}; the recording goes on and the session stays untranscribed until a system check covers them"
            ));
        }
        laps.lap("models");
        let mut queue =
            SessionQueue::open_with_clock(&dir, Arc::new(ist_text)).map_err(|e| e.to_string())?;
        laps.lap("session database");
        let job = queue
            .ensure_rolling_job(&name, models_json(&models), languages.to_vec(), clock_ms())
            .map_err(|e| e.to_string())?;
        laps.lap("rolling job");
        // A night job held for a later session before the app was closed is still held: the hold is the job's own state.
        let night_held = queue.night_job_is_held().unwrap_or(false);
        let fell_back = !queue.fallback_units().is_empty();
        let work_dir = dir.join("work");
        let handle = Arc::new(SessionHandle {
            name: name.clone(),
            queue: Mutex::new(queue),
            work_dir,
        });
        self.manager.add_session(handle.clone());
        laps.lap("manager");
        let logged_name = name.clone();
        plock(&self.open).push(OpenSession {
            name,
            dir,
            handle,
            job,
            languages: languages.to_vec(),
            submitted: BTreeSet::new(),
            finished: false,
            ended_ms: None,
            night_handled: false,
            night_cancelled: night_held,
            fell_back,
        });
        laps.lap("open-sessions list");
        self.ensure_slots(profile);
        laps.lap("engine slots");
        log_start_timing(&format!(
            "queue registration of {logged_name}: {}",
            laps.text()
        ));
        Ok(())
    }

    /// A unit of `session` was decoded on the processor without the Hinglish model after a graphics failure (it is flagged
    /// in the session folder already). The first one says what becomes of the session, from the night routing on the
    /// audio known now: redone with the Hinglish model at night, or marked "best finished on a faster machine" (the
    /// marker file is written now; the night pass decides again with the audio really recorded).
    fn note_without_hinglish(&self, session: &str) {
        let Some(profile) = self.profile() else {
            return;
        };
        let abandoned = self.manager.gpu_abandoned();
        let now = clock_ms();
        let mut open = plock(&self.open);
        let Some(s) = open.iter_mut().find(|s| s.name == session) else {
            return;
        };
        if s.fell_back {
            return;
        }
        s.fell_back = true;
        s.night_handled = false;
        let (total, mix) = {
            let q = plock(&s.handle.queue);
            let jobs = q.jobs().unwrap_or_default();
            let total = jobs
                .iter()
                .find(|j| j.kind == JobKind::Rolling)
                .and_then(|j| q.units(j.id).ok())
                .map_or(0, |u| u.iter().map(|u| u.end_ms).max().unwrap_or(0));
            (total, language_mix(&q, "post-1"))
        };
        let night_session = night_route::NightSession {
            name: s.name.clone(),
            languages: s.languages.clone(),
            audio: wb_queue::night::SessionAudio {
                id: s.name.clone(),
                audio_ms_by_lang: wb_queue::estimate::split_by_mix(total, &s.languages, &mix)
                    .into_iter()
                    .collect(),
            },
        };
        let redo_view = profile.for_redo(abandoned);
        let factor = wb_queue::speed::planning_factor(
            self.manager.speed_factor(true),
            wb_queue::speed::Pass::Night(redo_view.night_class()),
            2,
        );
        let start =
            wb_queue::night::day_end_reference_ms(PROGRAMME_JSON, now, night_wait_shift_ms() == 0)
                .saturating_add(wb_queue::night::START_AFTER_LAST_SESSION_MS)
                .max(now);
        let redo = night_route::project_redo(&profile, abandoned, start, &night_session, factor);
        if let Some(why) = &redo.why_not {
            let text =
                wb_queue::night::faster_machine_json(why, profile.hinglish_night_model(), now);
            if let Err(e) = std::fs::write(s.dir.join(wb_queue::night::FASTER_MACHINE_FILE), text) {
                self.core.push_log(format!(
                    "{session}: the faster-machine note could not be written: {e}"
                ));
            }
        }
        drop(open);
        self.core.push_log(redo.text.clone());
        let _ = self.app.state::<AppState>().events.send(Notice::event(
            NoticeLevel::Warning,
            "queue",
            redo.text,
        ));
    }

    /// A session started. A night pass planned before it is cancelled and planned again after the last session
    /// (owner, 19 Nov 2026: the programme had no entry for the day and the pass had been planned between two
    /// sessions): a night job that is not finished is held with the units it committed, and a night plan that was
    /// skipped is reconsidered with the new session's audio. A finished night pass stays.
    fn cancel_night_plans(&self, new_name: &str) {
        let now = clock_ms();
        let mut held: Vec<String> = Vec::new();
        let mut open = plock(&self.open);
        for s in open.iter_mut().filter(|s| s.name != new_name) {
            // The night job's own state decides, not `night_handled`: that flag is in memory and a restart resets it
            // (ASR audit of 20 Nov 2026), and a night job that is running must be held whatever the flag says.
            let night = plock(&s.handle.queue)
                .jobs()
                .unwrap_or_default()
                .into_iter()
                .find(|j| j.kind == JobKind::Night);
            match night {
                Some(j) if j.state == JobState::Done => {}
                Some(_) => {
                    if matches!(plock(&s.handle.queue).hold_night_job(now), Ok(true)) {
                        s.night_cancelled = true;
                        s.night_handled = false;
                        held.push(s.name.clone());
                    }
                }
                None => s.night_handled = false,
            }
        }
        drop(open);
        if !held.is_empty() {
            self.core.push_log(format!(
                "night pass cancelled: {new_name} started after it was planned; it is planned again after the last session (held: {})",
                held.join(", ")
            ));
        }
    }

    /// Hands every closed part not yet seen to the planner, and, once Stop was seen, asks for the final cut.
    fn submit_parts(&self, stopped_dir: Option<&str>, recording_dir: Option<&str>) {
        let mut open = plock(&self.open);
        let mut just_stopped: Vec<String> = Vec::new();
        for s in open.iter_mut() {
            let index = wb_recorder::parts::load(&s.dir).ok();
            if let Some(index) = index {
                for p in &index.parts {
                    if s.submitted.insert(p.part) {
                        self.submit_part(s, p);
                    }
                }
            }
            let is_stopped = session_has_stopped(stopped_dir, recording_dir, &s.dir);
            if is_stopped && !s.finished {
                s.finished = true;
                s.ended_ms = Some(clock_ms());
                self.manager.session_finished(&s.handle, s.job);
                just_stopped.push(s.name.clone());
            }
        }
        drop(open);
        // The first 'ready by' estimate after Stop, with what it rests on: the provisional detection allowance for
        // a session with several languages until one of its units has finished (owner, 17 Nov 2026).
        if !just_stopped.is_empty() {
            let ready = self.snapshot().ready_text;
            let basis = wb_queue::speed::basis_text(
                self.manager.speed_factor(true),
                self.manager.speed_units(true),
                self.manager.speed_factor(false),
                self.manager.speed_units(false),
            );
            for name in just_stopped {
                self.core
                    .push_log(format!("{name}: stopped. {ready} ({basis})"));
            }
        }
    }

    fn submit_part(&self, s: &OpenSession, p: &PartRecord) {
        let wav = s.dir.join(&p.file);
        self.manager.part_closed(
            &s.handle,
            s.job,
            p.part,
            p.start_ms,
            p.end_ms(),
            p.sample_rate,
            wav,
        );
    }

    /// One monitor step: the pause rules, the engines they stop, and the parts to plan.
    fn step(&self, last_pause: &mut Option<String>) {
        let (verdict, _transitions) = self.core.evaluate();
        self.executor.set_threads_fewer(verdict.fewer_threads);
        let now_pause = verdict.pause.map(|r| r.text().to_owned());
        if now_pause.is_some() && *last_pause != now_pause {
            // A rule began to hold: running engines stop now and resume from their staged files later.
            if let Some(why) = &now_pause {
                self.manager.cancel_running(why);
            }
        }
        if *last_pause != now_pause {
            self.app
                .state::<AppState>()
                .broadcast(serde_json::json!({ "kind": "scheduler_status" }));
            self.manager.poke();
        }
        *last_pause = now_pause;
    }
}

/// Starts the queue: the scheduler, the planner, the monitor and the sessions a restart left unfinished.
/// `parts_rx` is the feed's receiver: a closed part is only a prompt to re-read `parts.json`.
pub fn init(app: &AppHandle, feed: Arc<Feed>, parts_rx: Receiver<PartRecord>) {
    let Some(engines_dir) = crate::engines::locate_engines_dir() else {
        tracing::warn!("queue: no engines folder was found; post-processing is off");
        return;
    };
    let Some(models_dir) = crate::engines::locate_models_dir() else {
        tracing::warn!("queue: no models folder was found; post-processing is off");
        return;
    };
    let manifest = match wb_models::Manifest::load(&models_dir.join("manifest.json")) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("queue: the model manifest could not be read: {e}");
            return;
        }
    };
    let engines = crate::engines::discover(&engines_dir);
    let chunks_exe = ["avx2", "avx512"]
        .iter()
        .find_map(|v| engines.iter().find(|(n, _)| n == v).map(|(_, p)| p.clone()))
        .or_else(|| engines.first().map(|(_, p)| p.clone()));
    let Some(chunks_exe) = chunks_exe else {
        tracing::warn!("queue: no engine build was found; post-processing is off");
        return;
    };
    // Gate builds pass WB_* variables on to the engines on purpose (emulation needs them); every other build
    // removes them (row 102).
    let scrub_env = crate::engines::scrub_engine_env();

    let readings: Arc<AppReadings> = Arc::new(AppReadings {
        app: app.clone(),
        feed: feed.clone(),
        free: Mutex::new(None),
        profile_reads: std::sync::atomic::AtomicU64::new(0),
        profile_read_max_us: std::sync::atomic::AtomicU64::new(0),
        profile_read_total_us: std::sync::atomic::AtomicU64::new(0),
    });
    let core = Arc::new(Core::new(readings.clone()));
    let open: Arc<Mutex<Vec<OpenSession>>> = Arc::new(Mutex::new(Vec::new()));

    let profile_for_exec = readings.clone();
    let open_for_exec = open.clone();
    let executor = Arc::new(AppExecutor::new(Environment {
        models_dir: models_dir.clone(),
        manifest,
        engines,
        scrub_env,
        profile: Arc::new(move || profile_for_exec.profile()),
        contexts: Arc::new(move |name| {
            plock(&open_for_exec)
                .iter()
                .find(|s| s.name == name)
                .map(|s| SessionInfoForUnits {
                    dir: s.dir.clone(),
                    glossary: String::new(),
                    languages: s.languages.clone(),
                })
        }),
    }));
    let events: Arc<AppEvents> = Arc::new(AppEvents {
        app: app.clone(),
        core: core.clone(),
    });
    let gate = Arc::new(AppGate { core: core.clone() });
    let manager = Manager::new(
        gate,
        executor.clone(),
        events,
        Config {
            chunks_exe,
            models_dir,
            scrub_env,
            chunks_timeout: Duration::from_secs(600),
            gate_poll: Duration::from_secs(2),
            clock_ms: Arc::new(clock_ms),
            ist_text: Arc::new(ist_text),
        },
    );
    if let Err(e) = manager.start_planner() {
        tracing::error!("queue: the planner could not start: {e}");
        return;
    }
    let queue = Arc::new(Queue {
        app: app.clone(),
        core,
        manager,
        executor,
        open,
        slots_started: AtomicBool::new(false),
        night_switch_said: AtomicBool::new(false),
        feed,
        #[cfg(feature = "gate-hooks")]
        readings: readings.clone(),
    });
    app.manage(QueueState(queue.clone()));

    // Monitor: pause rules every step, `parts.json` every few seconds or when the recorder announced a part.
    let q = queue.clone();
    let spawned = std::thread::Builder::new()
        .name("wb-queue-monitor".into())
        .spawn(move || {
            let mut last_pause: Option<String> = None;
            let mut last_dropped = 0u64;
            let mut last_night = Instant::now();
            let mut last_parts = Instant::now()
                .checked_sub(PARTS_RECHECK)
                .unwrap_or_else(Instant::now);
            // Sessions left unfinished by a restart continue first.
            q.resume_unfinished();
            loop {
                q.step(&mut last_pause);
                let dropped = q.feed.dropped_parts();
                if dropped != last_dropped {
                    last_dropped = dropped;
                    q.core.push_log(format!(
                        "{dropped} closed-part notice(s) did not fit the queue's channel; parts.json is read for them"
                    ));
                }
                if last_night.elapsed() >= NIGHT_CHECK {
                    last_night = Instant::now();
                    q.night_step();
                    q.night_recheck();
                }
                let announced = parts_rx.try_iter().count() > 0;
                if announced || last_parts.elapsed() >= PARTS_RECHECK {
                    last_parts = Instant::now();
                    let stopped = q.stopped_session_dir();
                    let recording_dir = q.recording_session_dir();
                    q.submit_parts(stopped.as_deref(), recording_dir.as_deref());
                }
                std::thread::sleep(MONITOR_STEP);
            }
        });
    if let Err(e) = spawned {
        tracing::error!("queue: the monitor could not start: {e}");
    }
}

impl Queue {
    /// The folder of the session being recorded now, if any.
    fn recording_session_dir(&self) -> Option<String> {
        let st = self.app.state::<AppState>();
        if !st.recording.load(Ordering::SeqCst) {
            return None;
        }
        let snap = lock(&st.snapshot);
        snap.session.as_ref().map(|s| s.dir.clone())
    }

    /// The folder of the session whose recording has just ended (Stop was pressed), if one is open here.
    fn stopped_session_dir(&self) -> Option<String> {
        let st = self.app.state::<AppState>();
        let recording = st.recording.load(Ordering::SeqCst);
        let snap = lock(&st.snapshot);
        stopped_dir(
            recording,
            snap.session.as_ref().map(|s| s.dir.as_str()),
            snap.last_stop
                .as_ref()
                .map(|l| l.recorder.session_dir.as_str()),
        )
    }

    /// Sessions in the recordings folder with unfinished jobs (the app was closed or crashed): they continue
    /// from their cursors.
    fn resume_unfinished(&self) {
        let st = self.app.state::<AppState>();
        let root = lock(&st.settings).recordings_root();
        let Ok(entries) = std::fs::read_dir(&root) else {
            return;
        };
        for e in entries.flatten() {
            let dir = e.path();
            if !dir.join("transcript.db").is_file() {
                continue;
            }
            let Some(profile) = self.profile() else {
                return;
            };
            let Ok(q) = SessionQueue::open_with_clock(&dir, Arc::new(ist_text)) else {
                continue;
            };
            let unfinished = q.jobs().is_ok_and(|jobs| {
                jobs.iter()
                    .any(|j| matches!(j.state, JobState::Queued | JobState::Running))
            });
            let languages = q
                .jobs()
                .ok()
                .and_then(|j| j.into_iter().find(|j| j.kind == JobKind::Rolling))
                .map(|j| j.languages)
                .unwrap_or_default();
            drop(q);
            if !unfinished || languages.is_empty() {
                continue;
            }
            match self.register(dir.clone(), &languages, &profile) {
                Ok(()) => {
                    // A session whose recording ended before the restart gets its final cut.
                    let finished =
                        dir.join("session.json").is_file() && !self.is_recording_dir(&dir);
                    if finished {
                        let mut open = plock(&self.open);
                        if let Some(s) = open.iter_mut().find(|s| s.dir == dir)
                            && !s.finished
                        {
                            s.finished = true;
                            s.ended_ms = Some(clock_ms());
                            self.manager.session_finished(&s.handle, s.job);
                        }
                    }
                }
                Err(e) => tracing::warn!("queue: {}: {e}", dir.display()),
            }
        }
    }

    fn is_recording_dir(&self, dir: &std::path::Path) -> bool {
        let st = self.app.state::<AppState>();
        if !st.recording.load(Ordering::SeqCst) {
            return false;
        }
        lock(&st.snapshot)
            .session
            .as_ref()
            .is_some_and(|s| std::path::Path::new(&s.dir) == dir)
    }
}

impl Queue {
    /// The night pass (decision D6): once every rolling job of the day is done, nothing is recording and the last
    /// session ended at least 30 minutes ago, each session with audio in a language that has a night model gets a
    /// `post-2` job, if the estimate for all of them fits before 08:00 IST. Otherwise the pass is skipped and the
    /// reason is logged. It starts 30 minutes after the programme's last session to transcribe on the day (or after
    /// the last recorded session, when the programme does not list that day).
    fn night_step(&self) {
        use wb_queue::night::{self, Plan, SessionAudio};
        let Some(profile) = self.profile() else {
            return;
        };
        if profile.record_only {
            return;
        }
        let now = clock_ms();
        let recording = self
            .app
            .state::<AppState>()
            .recording
            .load(Ordering::SeqCst);
        let mut open = plock(&self.open);
        if open.is_empty() {
            return;
        }
        let mut rolling_pending = false;
        let mut last_end = 0u64;
        let mut candidates: Vec<usize> = Vec::new();
        for (i, s) in open.iter().enumerate() {
            let Some(end) = s.ended_ms else {
                rolling_pending = true;
                continue;
            };
            last_end = last_end.max(end);
            let q = plock(&s.handle.queue);
            let jobs = q.jobs().unwrap_or_default();
            match jobs
                .iter()
                .find(|j| j.kind == JobKind::Rolling)
                .map(|j| j.state)
            {
                Some(JobState::Done) => {
                    // A session whose night job was held for a new session is planned again with the rest (a held job
                    // whose last unit committed meanwhile is Done and needs nothing).
                    let night = jobs
                        .iter()
                        .find(|j| j.kind == JobKind::Night)
                        .map(|j| j.state);
                    let wants_night = match night {
                        None => true,
                        Some(JobState::Paused) => s.night_cancelled,
                        Some(_) => false,
                    };
                    // A session decoded with the Hinglish model in the rolling pass has no more accurate night model.
                    // (a session that fell back without the Hinglish model during the recording is redone with it)
                    if !s.night_handled
                        && wants_night
                        && (!profile.uses_hinglish(&s.languages) || s.fell_back)
                    {
                        candidates.push(i);
                    }
                }
                Some(JobState::Parked) | None => {}
                Some(_) => rolling_pending = true,
            }
        }
        // The pass waits for the programme's last session to transcribe on the day of the last recorded session,
        // not only for the sessions that happened to be recorded (a 30-minute gap between two sessions must not
        // start it early). A day the programme does not list goes by the recorded sessions alone, but not before
        // 20:00 IST (a session that ended at midday does not end the day); a programme entry overrides that.
        // The gate builds' shortened wait (`WB_GATE_NIGHT_WAIT_S`) turns the 20:00 rule off.
        let last_end =
            night::day_end_reference_ms(PROGRAMME_JSON, last_end, night_wait_shift_ms() == 0);
        if candidates.is_empty()
            || !night::may_start(
                now.saturating_add(night_wait_shift_ms()),
                recording,
                rolling_pending,
                last_end,
            )
        {
            return;
        }
        // A session that fell back is planned with the redo profile (the Hinglish model is its night model, on the card
        // or, after a failure in this run, on the processor where it was measured).
        let profile = if candidates.iter().any(|&i| open[i].fell_back) {
            profile.for_redo(self.manager.gpu_abandoned())
        } else {
            profile
        };
        let mut audio: Vec<night_route::NightSession> = Vec::new();
        for &i in &candidates {
            let s = &open[i];
            let q = plock(&s.handle.queue);
            let jobs = q.jobs().unwrap_or_default();
            let rolling_total = jobs
                .iter()
                .find(|j| j.kind == JobKind::Rolling)
                .and_then(|j| q.units(j.id).ok())
                .map_or(0, |u| u.iter().map(|u| u.end_ms).max().unwrap_or(0));
            // A night job held for a later session has committed some units: only the audio it has left counts
            // (the whole session would make the pass look longer than it is and could skip it).
            let total = jobs
                .iter()
                .find(|j| j.kind == JobKind::Night && j.state == JobState::Paused)
                .and_then(|j| q.remaining_audio_ms(j.id).ok())
                .unwrap_or(rolling_total);
            let mix = language_mix(&q, "post-1");
            audio.push(night_route::NightSession {
                name: s.name.clone(),
                languages: s.languages.clone(),
                audio: SessionAudio {
                    id: s.name.clone(),
                    audio_ms_by_lang: wb_queue::estimate::split_by_mix(total, &s.languages, &mix)
                        .into_iter()
                        .collect(),
                },
            });
        }
        // The profile's night speeds, moved to what this computer's finished units showed (the profile alone
        // promised 1.48 for a night unit that ran at 2.73 in the second smoke). Until a unit of a session with
        // several languages has finished, such a session is planned with the provisional emu-low detection
        // allowance (the profile's bench does no language detection). One session with several languages among
        // the candidates puts the whole plan on the safe side (the speeds are per language, not per session).
        let multi = candidates.iter().any(|&i| open[i].languages.len() > 1);
        // Only finished NIGHT units count (the day's rolling units ran another model); until one has finished every night
        // plan uses the night allowance, also a night of single-language sessions (owner, 27 Nov 2026; auditor's second run).
        let night_multi = self.manager.night_speed_factor(true);
        let night_single = self.manager.night_speed_factor(false);
        let factor = wb_queue::speed::night_planning_factor(
            night_multi,
            night_single,
            multi,
            profile.night_class(),
        );
        let measured = if multi {
            night_multi
        } else {
            night_single.or(night_multi)
        };
        // The Hinglish model decodes the night of the English + Hindi sessions when the plan made from the audio really
        // recorded finishes before 08:00; otherwise today's models run and those sessions are marked.
        let decision = night_route::decide(&profile, now, &audio, factor);
        for (name, why) in &decision.marks {
            let Some(s) = open.iter().find(|s| &s.name == name) else {
                continue;
            };
            self.core
                .push_log(format!("{name}: best finished on a faster machine: {why}"));
            let text = night::faster_machine_json(why, profile.hinglish_night_model(), now);
            if let Err(e) = std::fs::write(s.dir.join(night::FASTER_MACHINE_FILE), text) {
                self.core.push_log(format!(
                    "{name}: the faster-machine note could not be written: {e}"
                ));
            }
        }
        for (name, why) in &decision.dropped {
            self.core.push_log(format!("{name}: {why}"));
        }
        match decision.routed.plan.clone() {
            Plan::Run {
                sessions,
                estimate_s,
                finish_ms,
            } => {
                for (rank, name) in sessions.iter().enumerate() {
                    let Some(s) = open.iter().find(|s| &s.name == name) else {
                        continue;
                    };
                    let models = if decision.hinglish.contains(name) {
                        profile.hinglish_night_models(&s.languages)
                    } else {
                        profile.night_models(&s.languages)
                    };
                    let created = plock(&s.handle.queue).create_night_job(
                        &s.name,
                        models_json(&models),
                        s.languages.clone(),
                        now,
                    );
                    if let Err(e) = created {
                        self.core.push_log(format!(
                            "{}: the night pass could not be planned: {e}",
                            s.name
                        ));
                    } else {
                        // The plan's order is the order the night runs in, across sessions (the Hinglish redos first).
                        if let Err(e) = plock(&s.handle.queue).rank_night_job(rank) {
                            self.core.push_log(format!(
                                "{}: the night pass could not be ranked: {e}",
                                s.name
                            ));
                        }
                        if s.night_cancelled {
                            // Planned again: the held night job goes on from the units it has not committed.
                            let _ = plock(&s.handle.queue).resume_night_job(now);
                        }
                    }
                }
                // A session the priorities left out whose night job was held for a new session stays as it is, parked with the
                // reason (what it committed stays; its rolling transcript is the default revision), not Paused for ever.
                for &i in &candidates {
                    if open[i].night_cancelled && !sessions.contains(&open[i].name) {
                        let why = decision
                            .dropped
                            .iter()
                            .find(|(n, _)| n == &open[i].name)
                            .map_or_else(
                                || {
                                    "the night pass did not fit after the work with higher priority"
                                        .to_owned()
                                },
                                |(_, w)| w.clone(),
                            );
                        let _ = plock(&open[i].handle.queue).park_held_night_job(&why, now);
                    }
                }
                let basis = match measured {
                    Some(f) => format!(
                        "measured night speed: {f:.2} times the profile's, from {} finished night unit(s)",
                        self.manager.night_speed_units(multi)
                    ),
                    None => format!(
                        "provisional night allowance x{factor:.2}: no night unit has finished yet"
                    ),
                };
                self.core.push_log(format!(
                    "night pass planned for {} session(s), {} with the Hinglish model: about {:.1} h, ready by {} IST ({basis})",
                    sessions.len(),
                    decision.hinglish.len(),
                    estimate_s / 3600.0,
                    wb_queue::estimate::format_ist_hhmm(finish_ms)
                ));
                self.manager.poke();
            }
            Plan::Skip(reason) => {
                self.core
                    .push_log(format!("night pass skipped: {}", reason.text()));
                // A night job held for a later session would stay Paused with nobody to resume it: park it with
                // the reason (what it committed stays; the rolling transcript is still the default revision).
                for &i in &candidates {
                    if open[i].night_cancelled {
                        let _ = plock(&open[i].handle.queue).park_held_night_job(
                            &format!("night pass skipped: {}", reason.text()),
                            now,
                        );
                    }
                }
            }
        }
        for i in candidates {
            open[i].night_handled = true;
            open[i].night_cancelled = false;
        }
    }

    /// The night safety switch (owner, 26 Nov 2026; every night job since 27 Nov 2026). The night pass was planned before any
    /// night unit had run. Once night units have finished, this compares what every night job still has to decode, at the
    /// speed those units really ran, with 08:00 IST. If it will not finish, the jobs that have not started (no unit
    /// running, decoded or committed) are dropped from the lowest priority upwards (English, then Urdu): no night pass, the
    /// rolling transcript stands, and each drop is logged with its reason. If that is not enough, the Hinglish jobs that
    /// have not started go back to today's night models and the session is marked "best finished on a faster machine". A
    /// running unit is never stopped and a session that has started keeps its model.
    fn night_recheck(&self) {
        use wb_queue::night;
        let Some(profile) = self.profile() else {
            return;
        };
        if profile.record_only {
            return;
        }
        // Until a night unit has finished there is nothing to learn from: the plan made with the allowance stands. The
        // slower of the two measured kinds (several languages, one language) counts: a plan that is too short is the
        // dangerous error.
        let Some(factor) = [true, false]
            .into_iter()
            .filter_map(|multi| self.manager.night_speed_factor(multi))
            .reduce(f64::max)
        else {
            return;
        };
        let now = clock_ms();
        let open = plock(&self.open);
        // As `night_step`: a session that fell back is judged with the redo profile (the Hinglish model is its night model,
        // on the card or, after a failure in this run, on the processor where it was measured).
        let profile = if open.iter().any(|s| s.fell_back) {
            profile.for_redo(self.manager.gpu_abandoned())
        } else {
            profile
        };
        let hinglish_model = profile.hinglish_night_model().map(str::to_owned);
        let mut jobs: Vec<night_route::NightJob> = Vec::new();
        for s in open.iter() {
            let q = plock(&s.handle.queue);
            let Some(job) = q.jobs().unwrap_or_default().into_iter().find(|j| {
                j.kind == JobKind::Night && matches!(j.state, JobState::Queued | JobState::Running)
            }) else {
                continue;
            };
            // A job decodes with the Hinglish model when any of its models is one the manifest lists as such (also when this
            // profile no longer names a night model, as after a graphics failure whose redo uses the processor).
            let hinglish = job.models_by_lang.as_object().is_some_and(|m| {
                m.values().any(|v| {
                    v.as_str()
                        .is_some_and(|id| wb_capability::HINGLISH_MODELS.contains(&id))
                })
            });
            let started = q
                .units(job.id)
                .unwrap_or_default()
                .iter()
                .any(|u| u.state != UnitState::Pending);
            let remaining = q.remaining_audio_ms(job.id).unwrap_or(0);
            let mix = language_mix(&q, "post-1");
            jobs.push(night_route::NightJob {
                name: s.name.clone(),
                languages: s.languages.clone(),
                remaining_ms_by_lang: wb_queue::estimate::split_by_mix(
                    remaining,
                    &s.languages,
                    &mix,
                )
                .into_iter()
                .collect(),
                started,
                hinglish,
                rank: usize::try_from(
                    job.position
                        .saturating_sub(wb_runner::session_queue::NIGHT_RANK_BASE),
                )
                .unwrap_or(0),
            });
        }
        let Some(check) = night_route::recheck(&profile, now, &jobs, factor) else {
            return;
        };
        let reason = check.reason();
        let mut dropped_count = 0usize;
        for name in &check.drop {
            let Some(s) = open.iter().find(|s| &s.name == name) else {
                continue;
            };
            let was_hinglish = jobs.iter().any(|j| &j.name == name && j.hinglish);
            let why = format!(
                "no night pass: {reason}; this session had not started and has the lowest priority left, so the rolling transcript stands"
            );
            let dropped = plock(&s.handle.queue).drop_unstarted_night_job(&why, now);
            match dropped {
                Ok(true) => {
                    dropped_count += 1;
                    self.core.push_log(format!(
                        "night safety switch: {name} is dropped from the night ({why})"
                    ));
                    // The operator sees it: the rolling transcript is what the session keeps.
                    let _ = self.app.state::<AppState>().events.send(Notice::event(
                        NoticeLevel::Warning,
                        "queue",
                        format!("{name}: no night pass, the night ran slower than planned; the rolling transcript stands"),
                    ));
                    if was_hinglish {
                        let text =
                            night::faster_machine_json(&reason, hinglish_model.as_deref(), now);
                        if let Err(e) = std::fs::write(s.dir.join(night::FASTER_MACHINE_FILE), text)
                        {
                            self.core.push_log(format!(
                                "{name}: the faster-machine note could not be written: {e}"
                            ));
                        }
                    }
                }
                // The job started (or was parked) in between: it is not dropped and the log does not say it was.
                Ok(false) => {}
                Err(e) => self.core.push_log(format!(
                    "night safety switch: the night job of {name} could not be dropped: {e}"
                )),
            }
        }
        if check.switch.is_empty() && dropped_count == 0 {
            if !self.night_switch_said.swap(true, Ordering::SeqCst) {
                self.core.push_log(format!(
                    "night safety switch: {reason}; every session has started, so none is dropped or goes back to today's models"
                ));
            }
            return;
        }
        for name in &check.switch {
            let Some(s) = open.iter().find(|s| &s.name == name) else {
                continue;
            };
            let models = profile.night_models(&s.languages);
            let q = plock(&s.handle.queue);
            let Some(job) = q.jobs().unwrap_or_default().into_iter().find(|j| {
                j.kind == JobKind::Night && matches!(j.state, JobState::Queued | JobState::Running)
            }) else {
                continue;
            };
            drop(q);
            // A job that already decodes with exactly these models has nothing to go back to: no change, no marker.
            if job.models_by_lang == models_json(&models) {
                continue;
            }
            // One statement checks that no unit has started and changes the models: a job that started in between keeps the
            // model it started with.
            let changed =
                plock(&s.handle.queue).change_models_if_unstarted(job.id, &models_json(&models));
            match changed {
                Ok(false) => continue,
                Ok(true) => {
                    self.core.push_log(format!(
                        "night safety switch: {name} goes back to today's night models (the Hinglish night pass: {reason}); best finished on a faster machine"
                    ));
                    let text = night::faster_machine_json(&reason, hinglish_model.as_deref(), now);
                    if let Err(e) = std::fs::write(s.dir.join(night::FASTER_MACHINE_FILE), text) {
                        self.core.push_log(format!(
                            "{name}: the faster-machine note could not be written: {e}"
                        ));
                    }
                }
                Err(e) => self.core.push_log(format!(
                    "night safety switch: the models of {name} could not be changed: {e}"
                )),
            }
        }
        self.core.push_log(format!(
            "night safety switch: with {} night job(s) dropped and {} session(s) back on today's models the rest needs about {:.1} h (the night needed {:.1} h)",
            dropped_count,
            check.switch.len(),
            check.needed_after_s / 3600.0,
            check.needed_s / 3600.0
        ));
        self.manager.poke();
    }

    /// The profile in use, read fresh (`store::in_use`: safe defaults after a fingerprint change).
    fn profile(&self) -> Option<ProfileView> {
        crate::syscheck::profile_in_use(&self.app).map(|(cap, changes)| view_of(&cap, changes))
    }
}

// ----------------------------------------------------------------------------------- session start hook

/// Called by the command layer after the recorder started: creates the session's rolling job when the profile
/// in use allows processing at all. A failure here is reported and never touches the recording.
pub fn on_session_started(app: &AppHandle, dir: &str, expected: &[String], script: Option<&str>) {
    let Some(state) = app.try_state::<QueueState>() else {
        return;
    };
    let queue = state.0.clone();
    let mut laps = Laps::start();
    let languages = job_languages(expected, script);
    let dir = PathBuf::from(dir);
    log_language_warning(&queue, &dir, expected, script);
    let Some(profile) = queue.profile() else {
        queue.core.push_log(
            "no system check has run: the session is recorded and not transcribed yet".into(),
        );
        return;
    };
    if profile.record_only {
        queue.core.push_log(format!(
            "this computer only records: {}",
            profile
                .record_only_reason
                .as_deref()
                .unwrap_or("no transcription is planned here")
        ));
        return;
    }
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("session")
        .to_owned();
    laps.lap("profile read");
    match queue.register(dir, &languages, &profile) {
        Ok(()) => {
            laps.lap("registration");
            queue.cancel_night_plans(&name);
            laps.lap("night plans");
            log_start_timing(&format!(
                "queue start hook of {name}: {}; engine units running at the start: {}",
                laps.text(),
                queue.manager.running_count()
            ));
        }
        Err(e) => {
            let _ = app.state::<AppState>().events.send(Notice::event(
                NoticeLevel::Warning,
                "queue",
                e.clone(),
            ));
            queue.core.push_log(e);
        }
    }
}

// ------------------------------------------------------------------------------------------------ commands

/// One engine device working on a job: how many of its units run now or are committed, and the processor
/// threads its engine gets (a graphics engine takes at most 2; the scheduler lowers them after a second dropout).
#[derive(Debug, Clone, Serialize)]
pub struct JobDeviceRow {
    /// "cpu" or "gpu".
    pub device: String,
    pub threads: u32,
    pub units_running: usize,
    pub units_committed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobRow {
    pub id: i64,
    pub session: String,
    pub kind: String,
    pub revision: String,
    pub state: String,
    pub cursor_ms: u64,
    pub models_by_lang: serde_json::Value,
    pub languages: Vec<String>,
    pub units_total: usize,
    pub units_committed: usize,
    pub units_running: usize,
    pub audio_ms: u64,
    pub remaining_audio_ms: u64,
    pub error: Option<String>,
    pub position: i64,
    /// The devices the saved profile's configuration puts to work, with each one's share of this job.
    pub devices: Vec<JobDeviceRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchedulerRow {
    /// What differs between this computer and its saved check (safe defaults are in force when not empty).
    pub profile_changes: Vec<String>,
    pub paused: Option<String>,
    pub operator_paused: bool,
    pub free_ram_bytes: u64,
    pub fewer_threads: u32,
    pub log: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueSnapshot {
    pub jobs: Vec<JobRow>,
    pub scheduler: SchedulerRow,
    pub ready_text: String,
}

/// The devices of the configuration with their threads and this job's units on each. The thread rule is the
/// executor's: the profile's processor threads less the scheduler's reduction (at least 1), and a graphics
/// engine takes at most 2.
fn job_devices(
    profile: &ProfileView,
    fewer: u32,
    units: &[wb_queue::store::Unit],
) -> Vec<JobDeviceRow> {
    let Some(config) = profile.config else {
        return Vec::new();
    };
    let (cpu, gpu) = match config {
        DeviceConfig::Cpu => (true, false),
        DeviceConfig::Gpu { .. } => (false, true),
        DeviceConfig::Hybrid { .. } => (true, true),
    };
    let base = profile.cpu_threads.max(1).saturating_sub(fewer).max(1);
    let count = |device: Device, state: UnitState| {
        units
            .iter()
            .filter(|u| u.device == Some(device) && u.state == state)
            .count()
    };
    [
        (cpu, "cpu", Device::Cpu, base),
        (gpu, "gpu", Device::Gpu, base.min(2)),
    ]
    .into_iter()
    .filter(|(wanted, ..)| *wanted)
    .map(|(_, name, device, threads)| JobDeviceRow {
        device: name.to_owned(),
        threads,
        units_running: count(device, UnitState::Running),
        units_committed: count(device, UnitState::Committed),
    })
    .collect()
}

/// Milliseconds of speech in each language among a revision's committed segments.
fn language_mix(q: &SessionQueue, revision: &str) -> std::collections::BTreeMap<String, u64> {
    let mut mix = std::collections::BTreeMap::new();
    for seg in q
        .transcript()
        .store()
        .segments(revision, 0, u64::MAX)
        .unwrap_or_default()
    {
        *mix.entry(seg.lang).or_default() += seg.end_ms.saturating_sub(seg.start_ms);
    }
    mix
}

/// The factor the 'ready by' estimate multiplies a job's profile speeds by: the measured one for sessions of its
/// kind once a unit of one has finished, and until then the provisional emu-low detection allowance when the job
/// has more than one language (owner, 17 Nov 2026; replace the allowance with the laptops' measurements).
fn job_planning_factor(
    job: &wb_queue::store::Job,
    measured: Option<f64>,
    class: wb_queue::speed::DeviceClass,
) -> f64 {
    let pass = if job.kind == JobKind::Night {
        wb_queue::speed::Pass::Night(class)
    } else {
        wb_queue::speed::Pass::Rolling
    };
    wb_queue::speed::planning_factor(measured, pass, job.languages.len())
}

/// Seconds a job still needs: its remaining audio split over its languages by the mix committed so far, each
/// language at the profile's factor for the job's model for it on the configured device, moved to this computer's
/// measured speed (`factor`) once units have finished. Without a profile the audio counts at real time.
fn job_seconds_left(
    profile: Option<&ProfileView>,
    job: &wb_queue::store::Job,
    remaining_ms: u64,
    mix: &std::collections::BTreeMap<String, u64>,
    factor: Option<f64>,
) -> f64 {
    use wb_queue::speed::scaled;
    let Some(p) = profile else {
        return scaled(remaining_ms as f64 / 1000.0, factor);
    };
    let models = job.models_by_lang.as_object().cloned().unwrap_or_default();
    let by_lang: std::collections::BTreeMap<String, f64> = p
        .rtf_by_lang(&models, &job.languages, p.uses_gpu())
        .into_iter()
        .map(|(l, r)| (l, scaled(r, factor)))
        .collect();
    let slowest = by_lang.values().copied().fold(0.0, f64::max);
    let default = if slowest > 0.0 {
        slowest
    } else {
        scaled(1.0, factor)
    };
    let split = wb_queue::estimate::split_by_mix(remaining_ms, &job.languages, mix);
    wb_queue::estimate::job_seconds(&split, &by_lang, default, 0, 0.0)
}

fn state_name(s: JobState) -> &'static str {
    match s {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Paused => "paused",
        JobState::Done => "done",
        JobState::Parked => "parked",
    }
}

impl Queue {
    pub fn snapshot(&self) -> QueueSnapshot {
        let mut rows = Vec::new();
        let mut seconds = Vec::new();
        let open = plock(&self.open);
        let profile = self.profile();
        let verdict = self.core.verdict();
        for s in open.iter() {
            let q = plock(&s.handle.queue);
            let Ok(jobs) = q.jobs() else { continue };
            for j in jobs {
                let units = q.units(j.id).unwrap_or_default();
                let committed = units
                    .iter()
                    .filter(|u| u.state == UnitState::Committed)
                    .count();
                let running = units
                    .iter()
                    .filter(|u| u.state == UnitState::Running)
                    .count();
                let audio_ms = units.iter().map(|u| u.end_ms).max().unwrap_or(0);
                let remaining = q.remaining_audio_ms(j.id).unwrap_or(0);
                if matches!(j.state, JobState::Queued | JobState::Running) {
                    // This computer's measured speed for sessions of this kind once a unit of one has finished;
                    // until then the provisional detection allowance for several languages (owner, 17 Nov 2026).
                    let class = profile
                        .as_ref()
                        .map_or(wb_queue::speed::DeviceClass::Cpu, |p| p.night_class());
                    let factor = job_planning_factor(
                        &j,
                        self.manager.speed_factor(j.languages.len() > 1),
                        class,
                    );
                    seconds.push(job_seconds_left(
                        profile.as_ref(),
                        &j,
                        remaining,
                        &language_mix(&q, &j.revision),
                        Some(factor),
                    ));
                }
                let devices = profile
                    .as_ref()
                    .map(|p| job_devices(p, verdict.fewer_threads, &units))
                    .unwrap_or_default();
                rows.push(JobRow {
                    id: j.id,
                    session: j.session.clone(),
                    kind: j.kind.as_str().to_owned(),
                    revision: j.revision.clone(),
                    state: state_name(j.state).to_owned(),
                    cursor_ms: j.cursor_ms,
                    models_by_lang: j.models_by_lang.clone(),
                    languages: j.languages.clone(),
                    units_total: units.len(),
                    units_committed: committed,
                    units_running: running,
                    audio_ms,
                    remaining_audio_ms: remaining,
                    error: j.error.clone(),
                    position: j.position,
                    devices,
                });
            }
        }
        QueueSnapshot {
            jobs: rows,
            scheduler: SchedulerRow {
                profile_changes: profile
                    .as_ref()
                    .map(|p| p.changes.clone())
                    .unwrap_or_default(),
                paused: verdict.pause.map(|r| r.text().to_owned()),
                operator_paused: self.core.operator_paused(),
                free_ram_bytes: self.core.free_ram_bytes(),
                fewer_threads: verdict.fewer_threads,
                log: self
                    .core
                    .log()
                    .into_iter()
                    .rev()
                    .take(60)
                    .map(|l| l.text)
                    .collect(),
            },
            ready_text: wb_queue::estimate::ready_text(clock_ms(), &seconds),
        }
    }

    fn with_job<T>(
        &self,
        job: i64,
        f: impl FnOnce(&mut SessionQueue) -> Result<T, wb_runner::session_queue::Error>,
    ) -> Result<T, String> {
        let open = plock(&self.open);
        for s in open.iter() {
            let mut q = plock(&s.handle.queue);
            if q.jobs().is_ok_and(|jobs| jobs.iter().any(|j| j.id == job)) {
                return f(&mut q).map_err(|e| e.to_string());
            }
        }
        Err(format!("job {job} is not in an open session"))
    }
}

fn queue_of(app: &AppHandle) -> Result<Arc<Queue>, String> {
    app.try_state::<QueueState>()
        .map(|s| s.0.clone())
        .ok_or_else(|| {
            "Post-processing is not available: no engines or models were found.".to_owned()
        })
}

#[tauri::command]
pub async fn queue_snapshot(app: AppHandle) -> Result<QueueSnapshot, String> {
    blocking("queue_snapshot", move || Ok(queue_of(&app)?.snapshot())).await
}

#[tauri::command]
pub async fn queue_pause(app: AppHandle, job: Option<i64>) -> Result<(), String> {
    blocking("queue_pause", move || pause_now(&app, job)).await
}

/// What the `queue_pause` command does (the gate hook runs it too).
pub(crate) fn pause_now(app: &AppHandle, job: Option<i64>) -> Result<(), String> {
    let q = queue_of(app)?;
    match job {
        Some(id) => {
            q.with_job(id, |s| s.set_paused(id, true, clock_ms()))?;
            q.manager.cancel_running("the operator paused a job");
        }
        None => {
            q.core.set_operator_paused(true);
            q.manager.cancel_running("the operator paused the queue");
        }
    }
    q.core.push_log("the operator paused processing".into());
    Ok(())
}

#[tauri::command]
pub async fn queue_resume(app: AppHandle, job: Option<i64>) -> Result<(), String> {
    blocking("queue_resume", move || resume_now(&app, job)).await
}

/// What the `queue_resume` command does (the gate hook runs it too).
pub(crate) fn resume_now(app: &AppHandle, job: Option<i64>) -> Result<(), String> {
    let q = queue_of(app)?;
    match job {
        Some(id) => {
            q.with_job(id, |s| s.set_paused(id, false, clock_ms()))?;
        }
        None => q.core.set_operator_paused(false),
    }
    q.core.push_log("the operator resumed processing".into());
    q.manager.poke();
    Ok(())
}

#[tauri::command]
pub async fn queue_move_up(app: AppHandle, job: i64) -> Result<bool, String> {
    blocking("queue_move_up", move || {
        let q = queue_of(&app)?;
        let moved = q.with_job(job, |s| s.move_up(job))?;
        q.manager.poke();
        Ok(moved)
    })
    .await
}

#[tauri::command]
pub async fn queue_change_model(
    app: AppHandle,
    job: i64,
    models_by_lang: serde_json::Value,
) -> Result<(), String> {
    blocking("queue_change_model", move || {
        let q = queue_of(&app)?;
        q.with_job(job, |s| s.change_models(job, &models_by_lang))?;
        // Running engines were started with the old models: stop them; the remaining units use the new ones.
        q.manager.cancel_running("the models of a job were changed");
        q.manager.poke();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn set_language_lock(
    app: AppHandle,
    lang: Option<String>,
    audio_ms: u64,
) -> Result<(), String> {
    blocking("set_language_lock", move || {
        // Auditor item 13 of 20 Nov 2026: stamp the lock with the recorder's own latest audio time, which the backend
        // keeps, not with the time the screen last drew (up to one status tick earlier). The screen's value is the
        // fallback when the backend has no recorder status.
        let latest = lock(&app.state::<AppState>().snapshot)
            .recorder
            .as_ref()
            .map(|r| r.audio_ms);
        lock_language_now(&app, lang.as_deref(), lock_audio_ms(latest, audio_ms))
    })
    .await
}

/// The audio time a language lock is stamped with: the recorder's latest status when there is one, otherwise the
/// time the screen sent. Never earlier than the screen's, because the backend status is the newer of the two.
pub(crate) fn lock_audio_ms(recorder_audio_ms: Option<u64>, screen_audio_ms: u64) -> u64 {
    recorder_audio_ms.map_or(screen_audio_ms, |r| r.max(screen_audio_ms))
}

/// What the `set_language_lock` command does (the gate hook runs it too).
pub(crate) fn lock_language_now(
    app: &AppHandle,
    lang: Option<&str>,
    audio_ms: u64,
) -> Result<(), String> {
    let st = app.state::<AppState>();
    if !st.recording.load(Ordering::SeqCst) {
        return Err("A language can be locked only while a session is recording.".into());
    }
    let (dir, languages) = {
        let snap = lock(&st.snapshot);
        let s = snap.session.as_ref().ok_or("No session is open.")?;
        // A resumed session whose folder lists both Hindi and Urdu locks only among English and Hindi.
        (
            PathBuf::from(&s.dir),
            job_languages(&s.expected_languages, None),
        )
    };
    if let Some(l) = lang
        && !langlock::accepted(l, &languages)
    {
        return Err(format!("{l} is not one of this session's languages."));
    }
    langlock::append(&dir, audio_ms, lang, &ist_text())
        .map_err(|e| format!("The language change could not be saved: {e}"))?;
    tracing::info!(audio_ms, lang = ?lang, "language lock changed");
    Ok(())
}

#[tauri::command]
pub async fn import_session(app: AppHandle, source: String) -> Result<String, String> {
    blocking("import_session", move || import_now(&app, &source)).await
}

/// What the `import_session` command does (the gate hook runs it too).
pub(crate) fn import_now(app: &AppHandle, source: &str) -> Result<String, String> {
    let q = queue_of(app)?;
    let profile = q
        .profile()
        .ok_or("Run the system check first: this computer's abilities are not known yet.")?;
    wb_queue::import::refuse_on_record_only(profile.record_only).map_err(|e| e.to_string())?;
    let src = PathBuf::from(source);
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("The folder has no name.")?
        .to_owned();
    let root = lock(&app.state::<AppState>().settings).recordings_root();
    let dest = root.join(&name);
    wb_queue::import::import_session(&src, &dest).map_err(|e| e.to_string())?;
    // The imported session is processed like a recording that has ended.
    let expected = read_expected_languages(&dest);
    let languages = job_languages(&expected.0, expected.1.as_deref());
    log_language_warning(&q, &dest, &expected.0, expected.1.as_deref());
    q.register(dest.clone(), &languages, &profile)?;
    let mut open = plock(&q.open);
    if let Some(s) = open.iter_mut().find(|s| s.dir == dest) {
        s.finished = false;
    }
    drop(open);
    q.submit_parts(Some(&dest.to_string_lossy()), None);
    q.manager.poke();
    Ok(name)
}

/// `expected_languages` and the script from a session folder's `start-options.json` (a folder from another
/// machine may lack it: English and Hindi are the defaults).
fn read_expected_languages(dir: &std::path::Path) -> (Vec<String>, Option<String>) {
    let text = std::fs::read_to_string(dir.join("start-options.json")).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let langs: Vec<String> = v["expected_languages"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .filter(|l: &Vec<String>| !l.is_empty())
        .unwrap_or_else(|| vec!["en".into(), "hi".into()]);
    (langs, v["hindi_urdu_script"].as_str().map(str::to_owned))
}

#[tauri::command]
pub async fn session_transcript(
    app: AppHandle,
    session: String,
    revision: Option<String>,
) -> Result<Vec<Segment>, String> {
    blocking("session_transcript", move || {
        let q = queue_of(&app)?;
        let open = plock(&q.open);
        let s = open
            .iter()
            .find(|s| s.name == session)
            .ok_or_else(|| format!("{session} is not open in the queue"))?;
        let guard = plock(&s.handle.queue);
        let rev = revision.unwrap_or_else(|| "post-1".to_owned());
        guard
            .transcript()
            .store()
            .segments(&rev, 0, u64::MAX)
            .map_err(|e| e.to_string())
    })
    .await
}

/// What a gate run records about the queue at the end: every job and unit state, the scheduler log, the
/// configuration the queue worked in and whether it was forced.
#[cfg(feature = "gate-hooks")]
pub(crate) fn report_json(app: &AppHandle) -> serde_json::Value {
    let Some(q) = app.try_state::<QueueState>().map(|s| s.0.clone()) else {
        return serde_json::json!({ "error": "no queue" });
    };
    let snapshot = q.snapshot();
    serde_json::json!({
        "snapshot": snapshot,
        "log": q.core.log().into_iter().map(|l| format!("{} {}", l.at_ms, l.text)).collect::<Vec<_>>(),
        "configuration": q.profile().and_then(|p| p.config).map(|c| format!("{c:?}")),
        "forced_hybrid": gate_hooks::force_hybrid(),
        "speed_factor": q.manager.speed_factor(true),
        "speed_units": q.manager.speed_units(true),
        "speed_factor_one_language": q.manager.speed_factor(false),
        "speed_units_one_language": q.manager.speed_units(false),
        "estimate_basis": wb_queue::speed::basis_text(
            q.manager.speed_factor(true),
            q.manager.speed_units(true),
            q.manager.speed_factor(false),
            q.manager.speed_units(false),
        ),
        "detection_allowance_rolling": wb_queue::speed::ROLLING_DETECTION_ALLOWANCE,
        "detection_allowance_night": wb_queue::speed::NIGHT_ALLOWANCE_GPU,
        "detection_allowance_night_cpu": wb_queue::speed::NIGHT_ALLOWANCE_CPU,
        "profile_reads": q.readings.profile_reads.load(Ordering::Relaxed),
        "profile_read_max_us": q.readings.profile_read_max_us.load(Ordering::Relaxed),
        "profile_read_total_us": q.readings.profile_read_total_us.load(Ordering::Relaxed),
        "profile_read_mean_us": q
            .readings
            .profile_read_total_us
            .load(Ordering::Relaxed)
            .checked_div(q.readings.profile_reads.load(Ordering::Relaxed)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Auditor item 13: a language lock takes the recorder's latest audio time, and the screen's only when the
    /// backend has no recorder status.
    #[test]
    fn a_language_lock_is_stamped_with_the_recorders_latest_audio_time() {
        assert_eq!(lock_audio_ms(Some(61_250), 60_900), 61_250);
        assert_eq!(lock_audio_ms(None, 60_900), 60_900);
        assert_eq!(
            lock_audio_ms(Some(60_000), 60_900),
            60_900,
            "never earlier than what the screen showed"
        );
    }

    fn unit(idx: u32, device: Option<Device>, state: UnitState) -> wb_queue::store::Unit {
        wb_queue::store::Unit {
            id: i64::from(idx),
            job: 1,
            idx,
            start_ms: u64::from(idx) * 300_000,
            end_ms: u64::from(idx + 1) * 300_000,
            state,
            device,
            cpu_attempts: 0,
            gpu_attempts: 0,
            last_error: None,
            seam: None,
        }
    }

    fn profile_of(json: &str) -> ProfileView {
        ProfileView::from_capability(&wb_capability::parse_capability(json).unwrap(), vec![])
    }

    #[test]
    fn a_job_shows_the_devices_of_the_configuration_with_their_threads_and_units() {
        let hybrid = profile_of(include_str!(
            "../../../crates/wb-capability/fixtures/tier-c-mid-hybrid.json"
        ));
        let units = [
            unit(0, Some(Device::Gpu), UnitState::Committed),
            unit(1, Some(Device::Cpu), UnitState::Running),
            unit(2, Some(Device::Gpu), UnitState::Committed),
            unit(3, None, UnitState::Pending),
        ];
        let rows = job_devices(&hybrid, 0, &units);
        assert_eq!(
            rows.iter().map(|r| r.device.as_str()).collect::<Vec<_>>(),
            ["cpu", "gpu"]
        );
        assert_eq!(rows[0].threads, hybrid.cpu_threads.max(1));
        assert!(
            rows[1].threads <= 2,
            "a graphics engine takes at most 2 threads"
        );
        assert_eq!((rows[0].units_running, rows[0].units_committed), (1, 0));
        assert_eq!((rows[1].units_running, rows[1].units_committed), (0, 2));
        // The scheduler's reduction lowers the threads by one, never below one.
        let fewer = job_devices(&hybrid, 1, &units);
        assert_eq!(
            fewer[0].threads,
            hybrid.cpu_threads.max(1).saturating_sub(1).max(1)
        );
        assert_eq!(job_devices(&hybrid, 99, &units)[0].threads, 1);
    }

    fn rolling_job(models: serde_json::Value, languages: &[&str]) -> wb_queue::store::Job {
        wb_queue::store::Job {
            id: 1,
            session: "hall-a".into(),
            kind: JobKind::Rolling,
            models_by_lang: models,
            languages: languages.iter().map(|l| (*l).to_owned()).collect(),
            revision: "post-1".into(),
            state: JobState::Running,
            cursor_ms: 0,
            position: 1,
            created_ms: 0,
            started_ms: None,
            finished_ms: None,
            error: None,
        }
    }

    #[test]
    fn the_ready_estimate_uses_the_configured_device_the_language_mix_and_the_measured_speed() {
        let tier_g = profile_of(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ));
        assert!(tier_g.uses_gpu(), "the fixture is a graphics configuration");
        let models = tier_g.rolling_models(&["en".into(), "hi".into()]);
        let json = serde_json::to_value(&models).unwrap();
        let job = rolling_job(json, &["en", "hi"]);
        let mix = std::collections::BTreeMap::from([
            ("en".to_string(), 100_000u64),
            ("hi".to_string(), 300_000),
        ]);
        let hour = 3_600_000;
        let promised = job_seconds_left(Some(&tier_g), &job, hour, &mix, None);
        let by_lang = tier_g.rtf_by_lang(
            job.models_by_lang.as_object().unwrap(),
            &job.languages,
            true,
        );
        let expected = 3600.0 * (0.25 * by_lang["en"] + 0.75 * by_lang["hi"]);
        assert!(
            (promised - expected).abs() < 1e-6,
            "{promised} against {expected}"
        );
        // It does not take the slowest processor cell of the model when the configuration is graphics.
        let cpu_slowest = tier_g.rtf_for(&models["hi"], false);
        assert!(promised < 3600.0 * cpu_slowest || cpu_slowest == 0.0);
        // After units finished at twice the promised time the same remaining audio needs twice as long.
        let measured = job_seconds_left(Some(&tier_g), &job, hour, &mix, Some(2.0));
        assert!((measured - 2.0 * promised).abs() < 1e-6, "{measured}");
        // Without a profile the audio counts at real time, moved by the measured speed too.
        assert_eq!(job_seconds_left(None, &job, hour, &mix, None), 3600.0);
        assert_eq!(job_seconds_left(None, &job, hour, &mix, Some(1.5)), 5400.0);
    }

    #[test]
    fn the_first_ready_estimate_of_a_bilingual_session_carries_the_detection_allowance() {
        let tier_g = profile_of(include_str!(
            "../../../crates/wb-capability/fixtures/tier-g.json"
        ));
        let models = tier_g.rolling_models(&["en".into(), "hi".into()]);
        let json = serde_json::to_value(&models).unwrap();
        let hour = 3_600_000;
        let mix = std::collections::BTreeMap::from([("hi".to_string(), 1u64)]);
        let both = rolling_job(json.clone(), &["en", "hi"]);
        let one = rolling_job(json, &["hi"]);
        // Before any unit has finished: x1.34 for two languages, the profile's word for one.
        assert_eq!(
            job_planning_factor(&both, None, wb_queue::speed::DeviceClass::Gpu),
            wb_queue::speed::ROLLING_DETECTION_ALLOWANCE
        );
        assert_eq!(
            job_planning_factor(&one, None, wb_queue::speed::DeviceClass::Gpu),
            1.0
        );
        let promised = job_seconds_left(Some(&tier_g), &both, hour, &mix, Some(1.0));
        let first = job_seconds_left(
            Some(&tier_g),
            &both,
            hour,
            &mix,
            Some(job_planning_factor(
                &both,
                None,
                wb_queue::speed::DeviceClass::Gpu,
            )),
        );
        assert!(
            (first / promised - wb_queue::speed::ROLLING_DETECTION_ALLOWANCE).abs() < 1e-9,
            "{first} against {promised}"
        );
        // A night job gets the night figure.
        let mut night = rolling_job(serde_json::json!({}), &["en", "hi"]);
        night.kind = JobKind::Night;
        assert_eq!(
            job_planning_factor(&night, None, wb_queue::speed::DeviceClass::Gpu),
            wb_queue::speed::NIGHT_ALLOWANCE_GPU
        );
        // On a processor the night is planned at the same 1.5 until the laptops are measured (26 Nov 2026).
        assert_eq!(
            job_planning_factor(&night, None, wb_queue::speed::DeviceClass::Cpu),
            wb_queue::speed::NIGHT_ALLOWANCE_CPU
        );
        // Once a unit of such a session has finished, the measurement replaces the allowance.
        assert_eq!(
            job_planning_factor(&both, Some(1.46), wb_queue::speed::DeviceClass::Gpu),
            1.46
        );
    }

    #[test]
    fn a_processor_only_profile_has_one_device_row_and_no_profile_none() {
        let cpu = profile_of(include_str!(
            "../../../crates/wb-capability/fixtures/tier-c-cpu-only.json"
        ));
        let rows = job_devices(&cpu, 0, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].device, "cpu");
        let none = profile_of(include_str!(
            "../../../crates/wb-capability/fixtures/tier-r.json"
        ));
        assert!(
            job_devices(&none, 0, &[]).is_empty(),
            "tier R has no configuration"
        );
    }

    #[test]
    fn a_session_that_started_right_after_a_stop_does_not_hide_that_stop() {
        use std::path::Path;
        let parallel = Path::new("D:\\rec\\parallel-ii");
        let panel = "D:\\rec\\panel";
        // Start cleared `last_stop` and Panel is recording: Parallel II has stopped all the same (the old rule, which
        // looked at `last_stop` only, said it had not, and the session was never finalised).
        assert!(session_has_stopped(None, Some(panel), parallel));
        // The session that is recording has not stopped.
        assert!(!session_has_stopped(None, Some(panel), Path::new(panel)));
        // Nothing is recording and no Stop is known: nothing has stopped (a restart resumes such sessions elsewhere).
        assert!(!session_has_stopped(None, None, parallel));
        // The recorder's own `last_stop` still counts.
        assert!(session_has_stopped(
            Some("D:\\rec\\parallel-ii"),
            None,
            parallel
        ));
    }

    #[test]
    fn a_stop_is_seen_after_the_recorder_cleared_the_snapshots_session() {
        // What Stop leaves in the snapshot: no session, `last_stop` with the folder. (Before the fix the queue read
        // only `session`, so no Stop was ever seen and no session got its final cut.)
        assert_eq!(
            stopped_dir(false, None, Some("D:\\rec\\a")),
            Some("D:\\rec\\a".to_owned())
        );
        // The next session is already recording: the earlier stop is still reported.
        assert_eq!(
            stopped_dir(true, Some("D:\\rec\\b"), Some("D:\\rec\\a")),
            Some("D:\\rec\\a".to_owned())
        );
        // No stop yet.
        assert_eq!(stopped_dir(true, Some("D:\\rec\\a"), None), None);
        // The same folder is recording again (resumed after recovery): not a stop.
        assert_eq!(
            stopped_dir(true, Some("D:\\rec\\a"), Some("D:\\rec\\a")),
            None
        );
    }

    #[test]
    fn a_session_that_lists_both_hindi_and_urdu_is_decoded_in_hindi_and_the_log_names_it() {
        let expected = vec!["en".to_string(), "hi".to_string(), "ur".to_string()];
        // The script no longer picks Urdu for a list that names both (owner, 24 Nov 2026).
        for script in [None, Some("devanagari"), Some("nastaliq")] {
            assert_eq!(job_languages(&expected, script), vec!["en", "hi"]);
            assert!(
                language_warning("Panel", &expected, script)
                    .is_some_and(|w| w.starts_with("Panel lists both")),
            );
        }
        assert_eq!(
            job_languages(&["en".to_string(), "ur".to_string()], Some("nastaliq")),
            vec!["en", "ur"]
        );
        // A lock to Urdu is refused in such a session, and a lock to Hindi is accepted.
        let session = job_languages(&expected, None);
        assert!(!langlock::accepted("ur", &session));
        assert!(langlock::accepted("hi", &session));
        assert_eq!(
            job_languages(&["en".to_string()], Some("nastaliq")),
            vec!["en"]
        );
    }

    /// The scheduler's clock is the system clock in every build without the gate hooks, so the clock offset
    /// of the day simulation can never reach an installer build.
    #[cfg(not(feature = "gate-hooks"))]
    #[test]
    fn the_scheduler_clock_is_the_system_clock_in_a_build_without_gate_hooks() {
        let system = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let ours = clock_ms();
        assert!(ours.abs_diff(system) < 5_000, "{ours} against {system}");
    }
}
