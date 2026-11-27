//! One session's queue: its jobs, work units, in-order commit and rolling plan (Phase 4 step 4). No engine and no
//! Tauri in here: the app and the workers drive it, so every rule is a test with plain files.
//!
//! A unit goes `pending` -> `running` -> `decoded` -> `committed`. The engine writes the unit's segments to a
//! STAGED file (`work/<revision>/unit-NNNN.jsonl`, with chunk-end markers, so a kill resumes exactly). When the
//! engine reports the unit done its segments wait in the in-order buffer until every earlier unit is committed;
//! then they are appended to the revision's JSONL and fsynced, put in SQLite, and only then does the job's
//! cursor move and the unit become `committed` (the caller shows them in the UI after that, never before).
//!
//! After a crash [`SessionQueue::open`] makes the files and the database agree again: a `running` unit goes back
//! to `pending` (its staged file resumes it), a `decoded` unit is read from its staged file and queued for
//! commit, and a unit the cursor already covers is `committed`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wb_protocol::SegmentEvent;
use wb_queue::order::OrderBuffer;
use wb_queue::revision::Transcript;
use wb_queue::sched;
use wb_queue::store::{
    ATTEMPTS_PER_DEVICE, Device, Gap, Job, JobKind, JobState, NewJob, Retry, Segment, Unit,
    UnitState,
};
use wb_queue::units::{self, Part, Pause, Seam};
use wb_supervisor::Committer;

use crate::chunks::{PartPauses, merge_pauses};

/// A stretch with no segment of at least this long, and nothing in it the engine flagged, is recorded as a gap the
/// VAD called silence, so review can tell silence from loss (GATES row 106).
pub const GAP_MIN_MS: u64 = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Queue(#[from] wb_queue::Error),
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.display().to_string(),
        source,
    }
}

/// A closed part and the pauses `wb-engine chunks` found in it. Saved with the session so a restart does not
/// run `chunks` again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartEntry {
    pub part: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub sample_rate: u32,
    pub pauses: Vec<(u64, u64)>,
    pub fallback: Vec<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PlanState {
    parts: Vec<PartEntry>,
    /// Stop was pressed: the last unit ends at the end of the session.
    finished: bool,
}

/// What a worker is given to run.
#[derive(Debug, Clone)]
pub struct Claim {
    pub job: Job,
    pub unit: Unit,
    pub device: Device,
    /// The unit's staged file.
    pub staged: PathBuf,
}

/// What finishing a unit did.
#[derive(Debug, Default)]
pub struct Completed {
    /// Units committed by this call, in audio order (a later unit that was waiting for this one is among them).
    pub committed_units: Vec<u32>,
    /// The segments committed by this call, for the UI (shown only now, after the fsync).
    pub segments: Vec<Segment>,
    /// Segments the revision refused because they start before its committed end (a repeat). Always 0 in a
    /// healthy run; counted so a test and the log can say so.
    pub refused: usize,
    /// The job is complete: every unit committed and the plan closed.
    pub job_done: bool,
}

/// What a decoded unit waits with until every earlier unit is committed: its segments and the VAD gaps.
type UnitOutput = (Vec<Segment>, Vec<Gap>);

/// The time stamp (IST text) a segment reloaded from its staged file gets as `created_at`: the staged file keeps the engine's
/// segments, not when they were decoded, so the moment of the reload stands in and the field is never empty.
pub type Clock = Arc<dyn Fn() -> String + Send + Sync>;

pub struct SessionQueue {
    clock: Clock,
    dir: PathBuf,
    transcript: Transcript,
    orders: HashMap<i64, OrderBuffer<UnitOutput>>,
    plan: PlanState,
    params: units::Params,
}

fn staged_dir(dir: &Path, revision: &str) -> PathBuf {
    dir.join("work").join(revision)
}

pub fn staged_path(dir: &Path, revision: &str, idx: u32) -> PathBuf {
    staged_dir(dir, revision).join(format!("unit-{idx:04}.jsonl"))
}

/// The file in a session folder that lists the units decoded by the processor after the graphics engine failed, without
/// the Hinglish model (written by the app; the session is then redone at night or marked for a faster machine).
pub const FALLBACK_UNITS_FILE: &str = "fallback-units.json";

/// The queue position of the first-ranked night job; the positions of the other jobs of a session stay below it
/// (`Store::create_job` counts only positions under it).
pub use wb_queue::store::NIGHT_RANK_BASE;

fn plan_path(dir: &Path) -> PathBuf {
    dir.join("work").join("plan.json")
}

/// The segments of a staged file, up to its last chunk-end marker (anything after it is a chunk that was being
/// written and is cut off, as [`Committer::open_staged`] does).
pub fn read_staged(path: &Path) -> Result<Vec<SegmentEvent>> {
    // Opening cuts the unfinished chunk; the file is then exactly what was committed.
    let committer = Committer::open_staged(path).map_err(io(path))?;
    drop(committer);
    let text = std::fs::read_to_string(path).map_err(io(path))?;
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str::<SegmentEvent>(l).ok())
        .collect())
}

/// An engine segment as a transcript segment of `revision`, decoded on `device`.
pub fn to_segment(ev: &SegmentEvent, revision: &str, device: Device, created_at: &str) -> Segment {
    let flags = ev
        .flags
        .iter()
        .filter_map(|f| serde_json::to_value(f).ok())
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    Segment {
        revision: revision.to_owned(),
        start_ms: ev.start_ms,
        end_ms: ev.end_ms,
        text: ev.text.clone(),
        lang: ev.lang.clone(),
        lang_prob: ev.lang_prob,
        mixed: ev.mixed,
        dir: ev.dir.clone(),
        avg_logprob: ev.avg_logprob,
        no_speech_prob: ev.no_speech_prob,
        model: ev.model.clone(),
        device,
        flags,
        hint_text: ev.hint_text.clone(),
        created_at: created_at.to_owned(),
    }
}

/// The ranges of a unit that have no segment for at least [`GAP_MIN_MS`]: audio the VAD called non-speech. A
/// segment the engine flagged (unclear, repetition and the like) covers its range, so a dropped chunk is never
/// reported as silence.
pub fn vad_gaps(revision: &str, unit_start: u64, unit_end: u64, segments: &[Segment]) -> Vec<Gap> {
    let mut sorted: Vec<&Segment> = segments.iter().collect();
    sorted.sort_by_key(|s| (s.start_ms, s.end_ms));
    let mut gaps = Vec::new();
    let mut at = unit_start;
    let add = |from: u64, to: u64, gaps: &mut Vec<Gap>| {
        if to > from && to - from >= GAP_MIN_MS {
            gaps.push(Gap {
                revision: revision.to_owned(),
                start_ms: from,
                end_ms: to,
                kind: "vad_silence".into(),
            });
        }
    };
    for s in sorted {
        add(at, s.start_ms, &mut gaps);
        at = at.max(s.end_ms);
    }
    add(at, unit_end, &mut gaps);
    gaps
}

impl SessionQueue {
    /// Opens the session's queue, recovering from a crash. Segments reloaded from staged files get an empty `created_at`;
    /// the app uses [`SessionQueue::open_with_clock`].
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with_clock(dir, Arc::new(String::new))
    }

    /// As [`SessionQueue::open`], with the clock that stamps the segments reloaded from staged files (N6, auditor of 20 Nov 2026).
    pub fn open_with_clock(dir: &Path, clock: Clock) -> Result<Self> {
        let transcript = Transcript::open(dir)?;
        let plan = match std::fs::read(plan_path(dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Invalid(format!("{}: {e}", plan_path(dir).display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PlanState::default(),
            Err(e) => return Err(io(&plan_path(dir))(e)),
        };
        let mut q = Self {
            clock,
            dir: dir.to_owned(),
            transcript,
            orders: HashMap::new(),
            plan,
            params: units::Params::default(),
        };
        q.recover()?;
        Ok(q)
    }

    fn recover(&mut self) -> Result<()> {
        let jobs = self.transcript.store().jobs()?;
        for job in jobs {
            let order = self.rebuild_order(&job, true)?;
            if matches!(job.state, JobState::Queued | JobState::Running) {
                self.orders.insert(job.id, order);
            }
        }
        Ok(())
    }

    /// The order buffer of a job, built from what the store says: its next unit is the first one that is not
    /// committed, and every unit decoded but not yet committed is reloaded from its staged file. Used at start-up
    /// (`reset_running`: nothing is running, so a unit left Running is Pending again), when a parked job is resumed,
    /// and when a unit finishes for a job whose buffer was dropped. A park drops the buffer while the other engine
    /// may still be decoding; a buffer started at the unit that finishes then would commit it before an earlier unit
    /// that is not committed, move the cursor past that unit and lose its text (ASR audit of 21 Nov 2026).
    fn rebuild_order(&mut self, job: &Job, reset_running: bool) -> Result<OrderBuffer<UnitOutput>> {
        let stamp = (self.clock)();
        let units = self.transcript.store().units(job.id)?;
        // Units the cursor already covers were committed: the crash came between the cursor and the state.
        for u in &units {
            if u.end_ms <= job.cursor_ms && u.state != UnitState::Committed {
                self.transcript.store_mut().set_unit_state(
                    job.id,
                    u.idx,
                    UnitState::Committed,
                    None,
                )?;
            }
        }
        let units = self.transcript.store().units(job.id)?;
        let next = units
            .iter()
            .find(|u| u.state != UnitState::Committed)
            .map_or(units.iter().map(|u| u.idx + 1).max().unwrap_or(0), |u| {
                u.idx
            });
        let mut order = OrderBuffer::new(next, job.cursor_ms);
        for u in &units {
            match u.state {
                UnitState::Running if reset_running => {
                    self.transcript.store_mut().set_unit_state(
                        job.id,
                        u.idx,
                        UnitState::Pending,
                        None,
                    )?;
                }
                UnitState::Decoded => {
                    let path = staged_path(&self.dir, &job.revision, u.idx);
                    let device = u.device.unwrap_or(Device::Cpu);
                    match read_staged(&path) {
                        Ok(events) if !events.is_empty() || path.exists() => {
                            let segs: Vec<Segment> = events
                                .iter()
                                .map(|e| to_segment(e, &job.revision, device, &stamp))
                                .collect();
                            let gaps = vad_gaps(&job.revision, u.start_ms, u.end_ms, &segs);
                            // `idx` is at or after `next`: units before it are committed.
                            if u.idx >= order.next_idx() {
                                let _ = order.finished(u.idx, u.end_ms, (segs, gaps));
                            }
                        }
                        _ => {
                            // The staged file is gone: decode the unit again.
                            self.transcript.store_mut().set_unit_state(
                                job.id,
                                u.idx,
                                UnitState::Pending,
                                None,
                            )?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(order)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        Ok(self.transcript.store().jobs()?)
    }

    pub fn units(&self, job: i64) -> Result<Vec<Unit>> {
        Ok(self.transcript.store().units(job)?)
    }

    // ---- jobs ----

    /// The session's rolling job (the one writing `post-1`), created on first use.
    pub fn ensure_rolling_job(
        &mut self,
        session: &str,
        models_by_lang: serde_json::Value,
        languages: Vec<String>,
        now_ms: u64,
    ) -> Result<i64> {
        if let Some(j) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Rolling && j.revision == "post-1")
        {
            return Ok(j.id);
        }
        let id = self.transcript.store_mut().create_job(&NewJob {
            session: session.to_owned(),
            kind: JobKind::Rolling,
            models_by_lang,
            languages,
            revision: "post-1".into(),
            created_ms: now_ms,
        })?;
        self.orders.insert(id, OrderBuffer::new(0, 0));
        Ok(id)
    }

    /// A night job (`post-2`) over the same unit ranges as the rolling job, once that is complete.
    pub fn create_night_job(
        &mut self,
        session: &str,
        models_by_lang: serde_json::Value,
        languages: Vec<String>,
        now_ms: u64,
    ) -> Result<i64> {
        let rolling = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Rolling && j.revision == "post-1")
            .ok_or_else(|| Error::Invalid("no rolling job to copy the units from".into()))?;
        if rolling.state != JobState::Done {
            return Err(Error::Invalid(
                "the night pass starts after the rolling pass is complete".into(),
            ));
        }
        if let Some(j) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
        {
            return Ok(j.id);
        }
        let id = self.transcript.store_mut().create_job(&NewJob {
            session: session.to_owned(),
            kind: JobKind::Night,
            models_by_lang,
            languages,
            revision: "post-2".into(),
            created_ms: now_ms,
        })?;
        for u in self.transcript.store().units(rolling.id)? {
            self.transcript.store_mut().add_unit(
                id,
                u.idx,
                u.start_ms,
                u.end_ms,
                u.seam.as_deref(),
            )?;
        }
        self.orders.insert(id, OrderBuffer::new(0, 0));
        self.plan.finished = true;
        Ok(id)
    }

    /// Flags a unit as decoded without the Hinglish model after a graphics failure (owner, 27 Nov 2026): one entry per
    /// (job, unit), kept in [`FALLBACK_UNITS_FILE`] of the session folder, written whole and renamed into place.
    pub fn flag_fallback_unit(
        &self,
        job: i64,
        unit: u32,
        models: &serde_json::Value,
        now_ms: u64,
    ) -> Result<()> {
        let path = self.dir.join(FALLBACK_UNITS_FILE);
        let mut doc = self.fallback_doc();
        let units = doc["units"]
            .as_array_mut()
            .ok_or_else(|| Error::Invalid(format!("{} has no unit list", path.display())))?;
        if units
            .iter()
            .any(|u| u["job"].as_i64() == Some(job) && u["unit"].as_u64() == Some(u64::from(unit)))
        {
            return Ok(());
        }
        units.push(serde_json::json!({
            "job": job, "unit": unit, "models": models, "flagged_at_ms": now_ms,
        }));
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&doc).map_err(wb_queue::Error::from)?;
        let mut file = std::fs::File::create(&tmp).map_err(io(&tmp))?;
        std::io::Write::write_all(&mut file, &bytes).map_err(io(&tmp))?;
        file.sync_all().map_err(io(&tmp))?;
        drop(file);
        std::fs::rename(&tmp, &path).map_err(io(&path))?;
        Ok(())
    }

    fn fallback_doc(&self) -> serde_json::Value {
        std::fs::read(self.dir.join(FALLBACK_UNITS_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .filter(|v| v["units"].is_array())
            .unwrap_or_else(|| {
                serde_json::json!({
                    "version": 1,
                    "note": "decoded without the Hinglish model after a graphics failure",
                    "units": [],
                })
            })
    }

    /// The units flagged by [`Self::flag_fallback_unit`], as `(job, unit)`.
    pub fn fallback_units(&self) -> Vec<(i64, u32)> {
        self.fallback_doc()["units"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|u| {
                        Some((u["job"].as_i64()?, u32::try_from(u["unit"].as_u64()?).ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The note a held night job carries in its error column. A hold is the job's own persisted state (Paused with this
    /// note), so it survives a restart of the app: an operator's Pause leaves the column empty and is not a hold
    /// (item 3 of the auditor of 20 Nov 2026, owner's decision of 27 Nov 2026).
    pub const NIGHT_HELD_NOTE: &'static str = "held: another session started after the night pass was planned; it is planned again after the last session";

    /// The night pass of this session is cancelled because another session started after it was planned (owner,
    /// 19 Nov 2026): the night job is held, with every unit it already committed, until the pass is planned again
    /// after the last session. True when a night job that had not finished was held.
    pub fn hold_night_job(&mut self, now_ms: u64) -> Result<bool> {
        let Some(night) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
        else {
            return Ok(false);
        };
        if !matches!(night.state, JobState::Queued | JobState::Running) {
            return Ok(false);
        }
        self.set_paused(night.id, true, now_ms)?;
        self.transcript.store_mut().set_job_state(
            night.id,
            JobState::Paused,
            Some(Self::NIGHT_HELD_NOTE),
            now_ms,
        )?;
        Ok(true)
    }

    /// The night job of this session is held for a later session (as [`Self::hold_night_job`] left it, possibly before
    /// the app was restarted): Paused with the hold's note.
    pub fn night_job_is_held(&self) -> Result<bool> {
        Ok(self.transcript.store().jobs()?.into_iter().any(|j| {
            j.kind == JobKind::Night
                && j.state == JobState::Paused
                && j.error.as_deref() == Some(Self::NIGHT_HELD_NOTE)
        }))
    }

    /// Ranks this session's night job among the sessions' night jobs (0 first): the manager takes the job with the
    /// lowest `(priority, position)` across sessions, and every session's own queue numbers its jobs from 1, so without a
    /// rank the night would run in recording order whatever the plan's priorities were (auditor, 27 Nov 2026). The
    /// positions of the other kinds stay below [`NIGHT_RANK_BASE`]. True when the session has a night job.
    pub fn rank_night_job(&mut self, rank: usize) -> Result<bool> {
        let Some(night) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
        else {
            return Ok(false);
        };
        let position = NIGHT_RANK_BASE + i64::try_from(rank).unwrap_or(i64::MAX - NIGHT_RANK_BASE);
        self.transcript
            .store_mut()
            .set_job_position(night.id, position)?;
        Ok(true)
    }

    /// The night pass was held for a new session and the plan made after the last session does not run it (the
    /// pass does not fit): the held job is parked with the reason, so it does not stay Paused with nobody to resume it.
    /// What it committed stays; the rolling transcript remains the default revision. True when a held job was parked.
    pub fn park_held_night_job(&mut self, reason: &str, now_ms: u64) -> Result<bool> {
        let Some(night) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
        else {
            return Ok(false);
        };
        if night.state != JobState::Paused {
            return Ok(false);
        }
        self.transcript.store_mut().set_job_state(
            night.id,
            JobState::Parked,
            Some(reason),
            now_ms,
        )?;
        Ok(true)
    }

    /// The night safety switch drops this session's night job (owner, 27 Nov 2026): the night runs slower than planned and
    /// the job, which has not started (no unit running, decoded or committed), would not finish before 08:00 IST. The job is
    /// parked with the reason, the rolling transcript stays the default revision, and a running unit is never stopped. The
    /// check and the change happen under one hold of the session's queue, so a unit cannot be claimed in between. True when
    /// a job was parked; false when there is none, or it has started, or it is not queued or running (Paused, Parked, Done).
    pub fn drop_unstarted_night_job(&mut self, reason: &str, now_ms: u64) -> Result<bool> {
        let Some(night) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
        else {
            return Ok(false);
        };
        if !matches!(night.state, JobState::Queued | JobState::Running) {
            return Ok(false);
        }
        let started = self
            .transcript
            .store()
            .units(night.id)?
            .iter()
            .any(|u| u.state != UnitState::Pending);
        if started {
            return Ok(false);
        }
        self.park_job(night.id, reason, now_ms)?;
        Ok(true)
    }

    /// The night job held by [`Self::hold_night_job`] goes on when the pass is planned again: its committed units
    /// are not decoded twice. True when a held night job was resumed.
    pub fn resume_night_job(&mut self, now_ms: u64) -> Result<bool> {
        let Some(night) = self
            .transcript
            .store()
            .jobs()?
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
        else {
            return Ok(false);
        };
        if night.state != JobState::Paused {
            return Ok(false);
        }
        self.set_paused(night.id, false, now_ms)?;
        Ok(true)
    }

    pub fn set_paused(&mut self, job: i64, paused: bool, now_ms: u64) -> Result<()> {
        let j = self
            .transcript
            .store()
            .job(job)?
            .ok_or_else(|| Error::Invalid(format!("no job {job}")))?;
        let next = match (j.state, paused) {
            (JobState::Queued | JobState::Running, true) => JobState::Paused,
            (JobState::Paused, false) => JobState::Queued,
            (JobState::Parked, false) => JobState::Queued,
            (s, _) => s,
        };
        if next != j.state {
            self.transcript
                .store_mut()
                .set_job_state(job, next, None, now_ms)?;
            if next == JobState::Queued {
                self.reopen_order(job)?;
            }
        }
        Ok(())
    }

    /// A job goes back to the queue: its order buffer is rebuilt when a park dropped it (units decoded but not
    /// committed come back from their staged files), and a unit that failed gets another try (the operator asked
    /// to resume). The buffer may exist already: a unit that finished after the park rebuilt it.
    fn reopen_order(&mut self, job: i64) -> Result<()> {
        let j = self
            .transcript
            .store()
            .job(job)?
            .ok_or_else(|| Error::Invalid(format!("no job {job}")))?;
        if !self.orders.contains_key(&job) {
            let order = self.rebuild_order(&j, false)?;
            self.orders.insert(job, order);
        }
        let units = self.transcript.store().units(job)?;
        for u in units.iter().filter(|u| u.state == UnitState::Failed) {
            self.transcript.store_mut().retry_unit(job, u.idx)?;
        }
        Ok(())
    }

    pub fn move_up(&mut self, job: i64) -> Result<bool> {
        Ok(self.transcript.store_mut().move_up(job)?)
    }

    /// Changes the models of a job only if no unit of it has started (one statement: a unit cannot be claimed in between).
    /// True when the models were changed.
    pub fn change_models_if_unstarted(
        &mut self,
        job: i64,
        models_by_lang: &serde_json::Value,
    ) -> Result<bool> {
        Ok(self
            .transcript
            .store_mut()
            .set_models_if_unstarted(job, models_by_lang)?)
    }

    /// Change Model: the remaining units use the new models; committed units keep what decoded them.
    pub fn change_models(&mut self, job: i64, models_by_lang: &serde_json::Value) -> Result<()> {
        Ok(self
            .transcript
            .store_mut()
            .set_models(job, models_by_lang)?)
    }

    // ---- the rolling plan ----

    fn save_plan(&self) -> Result<()> {
        let path = plan_path(&self.dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.plan).map_err(wb_queue::Error::from)?;
        // Written and flushed through one handle: on Windows `sync_all` needs write access, which a plain `open`
        // does not give.
        let mut file = std::fs::File::create(&tmp).map_err(io(&tmp))?;
        std::io::Write::write_all(&mut file, &bytes).map_err(io(&tmp))?;
        file.sync_all().map_err(io(&tmp))?;
        drop(file);
        std::fs::rename(&tmp, &path).map_err(io(&path))?;
        Ok(())
    }

    /// A part closed, and its pauses are known. Adding the same part again replaces it.
    pub fn add_part(
        &mut self,
        part: u32,
        start_ms: u64,
        end_ms: u64,
        sample_rate: u32,
        pauses: &PartPauses,
    ) -> Result<()> {
        let entry = PartEntry {
            part,
            start_ms,
            end_ms,
            sample_rate,
            pauses: pauses
                .pauses
                .iter()
                .map(|p| (p.start_ms, p.end_ms))
                .collect(),
            fallback: pauses.fallback.clone(),
        };
        match self.plan.parts.iter_mut().find(|p| p.part == part) {
            Some(existing) => *existing = entry,
            None => self.plan.parts.push(entry),
        }
        self.plan.parts.sort_by_key(|p| p.part);
        self.save_plan()
    }

    /// The parts whose pauses are already known (a restart does not find them again).
    pub fn known_parts(&self) -> Vec<u32> {
        self.plan.parts.iter().map(|p| p.part).collect()
    }

    pub fn plan_parts(&self) -> &[PartEntry] {
        &self.plan.parts
    }

    pub fn finished(&self) -> bool {
        self.plan.finished
    }

    /// Cuts the audio not yet covered by a unit into new units and adds them to `job`. Returns how many. With
    /// `finished` (Stop) the last unit ends at the end of the session and the plan is closed.
    pub fn plan_units(&mut self, job: i64, finished: bool) -> Result<usize> {
        let existing = self.transcript.store().units(job)?;
        let from = existing.iter().map(|u| u.end_ms).max().unwrap_or(0);
        let next_idx = existing.iter().map(|u| u.idx + 1).max().unwrap_or(0);
        let parts: Vec<Part> = self
            .plan
            .parts
            .iter()
            .map(|p| Part {
                start_ms: p.start_ms,
                end_ms: p.end_ms,
                sample_rate: p.sample_rate,
            })
            .collect();
        let mut pauses: Vec<Pause> = self
            .plan
            .parts
            .iter()
            .flat_map(|p| {
                p.pauses.iter().map(|(s, e)| Pause {
                    start_ms: *s,
                    end_ms: *e,
                })
            })
            .collect();
        merge_pauses(&mut pauses);
        let fallback: Vec<u64> = self
            .plan
            .parts
            .iter()
            .flat_map(|p| p.fallback.iter().copied())
            .collect();
        let specs = units::plan(
            next_idx,
            from,
            &parts,
            &pauses,
            &fallback,
            finished,
            &self.params,
        );
        for s in &specs {
            self.transcript.store_mut().add_unit(
                job,
                s.idx,
                s.start_ms,
                s.end_ms,
                s.seam.map(Seam::as_str),
            )?;
        }
        if finished && !self.plan.finished {
            self.plan.finished = true;
            self.save_plan()?;
        }
        self.finalize_if_complete(job, 0)?;
        Ok(specs.len())
    }

    // ---- running units ----

    /// The job and unit a free engine on `device` would run next: the best-priority runnable job, its first
    /// pending unit this device may still try.
    fn best(&self, device: Device) -> Result<Option<(Job, Unit)>> {
        let jobs = self.transcript.store().jobs()?;
        let mut ready: HashMap<i64, Unit> = HashMap::new();
        for j in &jobs {
            if !matches!(j.state, JobState::Queued | JobState::Running) {
                continue;
            }
            if let Some(u) =
                self.transcript.store().units(j.id)?.into_iter().find(|u| {
                    u.state == UnitState::Pending && u.attempts(device) < ATTEMPTS_PER_DEVICE
                })
            {
                ready.insert(j.id, u);
            }
        }
        let Some(job) = sched::pick_job(&jobs, |j| ready.contains_key(&j.id)).cloned() else {
            return Ok(None);
        };
        Ok(ready.remove(&job.id).map(|u| (job, u)))
    }

    /// `(priority, queue position)` of the work a free engine on `device` would take, to choose between sessions.
    pub fn peek(&self, device: Device) -> Result<Option<(u8, i64)>> {
        Ok(self
            .best(device)?
            .map(|(j, _)| (sched::priority(j.kind), j.position)))
    }

    /// The next unit a free engine on `device` should run. The unit is marked `running`.
    pub fn claim(&mut self, device: Device, now_ms: u64) -> Result<Option<Claim>> {
        let Some((job, mut unit)) = self.best(device)? else {
            return Ok(None);
        };
        self.transcript.store_mut().set_unit_state(
            job.id,
            unit.idx,
            UnitState::Running,
            Some(device),
        )?;
        if job.state != JobState::Running {
            self.transcript
                .store_mut()
                .set_job_state(job.id, JobState::Running, None, now_ms)?;
        }
        let staged = staged_path(&self.dir, &job.revision, unit.idx);
        unit.state = UnitState::Running;
        unit.device = Some(device);
        Ok(Some(Claim {
            job,
            unit,
            device,
            staged,
        }))
    }

    /// Parks a job with the reason (a part whose pauses could not be found, say): the Queue row shows it and
    /// the operator can resume it.
    pub fn park_job(&mut self, job: i64, error: &str, now_ms: u64) -> Result<()> {
        self.transcript
            .store_mut()
            .set_job_state(job, JobState::Parked, Some(error), now_ms)?;
        self.orders.remove(&job);
        Ok(())
    }

    /// The engine was stopped (a pause rule, the operator) before the unit was done: it is pending again and its
    /// staged file resumes it.
    pub fn release(&mut self, claim: &Claim) -> Result<()> {
        self.transcript.store_mut().set_unit_state(
            claim.job.id,
            claim.unit.idx,
            UnitState::Pending,
            None,
        )?;
        Ok(())
    }

    /// The unit failed (the supervisor gave up on it). Three failures on one device move it to the other, then
    /// the job is parked with the error.
    pub fn fail(
        &mut self,
        claim: &Claim,
        error: &str,
        other_device_available: bool,
        now_ms: u64,
    ) -> Result<Retry> {
        let retry = self.transcript.store_mut().record_failure(
            claim.job.id,
            claim.unit.idx,
            claim.device,
            error,
            other_device_available,
        )?;
        if retry == Retry::GiveUp {
            self.transcript.store_mut().set_job_state(
                claim.job.id,
                JobState::Parked,
                Some(error),
                now_ms,
            )?;
            self.orders.remove(&claim.job.id);
        }
        Ok(retry)
    }

    /// The engine finished the unit: its staged segments wait for every earlier unit, then are committed in audio
    /// order.
    pub fn complete(&mut self, claim: &Claim, created_at: &str, now_ms: u64) -> Result<Completed> {
        let events = read_staged(&claim.staged)?;
        let segments: Vec<Segment> = events
            .iter()
            .map(|e| to_segment(e, &claim.job.revision, claim.device, created_at))
            .collect();
        let gaps = vad_gaps(
            &claim.job.revision,
            claim.unit.start_ms,
            claim.unit.end_ms,
            &segments,
        );
        let job_id = claim.job.id;
        if !self.orders.contains_key(&job_id) {
            // The job was parked while this unit was decoding: the buffer starts at the first unit that is not
            // committed, never at this one (built before this unit is marked Decoded, so it is not loaded twice).
            let j = self
                .transcript
                .store()
                .job(job_id)?
                .ok_or_else(|| Error::Invalid(format!("no job {job_id}")))?;
            let order = self.rebuild_order(&j, false)?;
            self.orders.insert(job_id, order);
        }
        self.transcript.store_mut().set_unit_state(
            job_id,
            claim.unit.idx,
            UnitState::Decoded,
            Some(claim.device),
        )?;
        let order = self
            .orders
            .get_mut(&job_id)
            .ok_or_else(|| Error::Invalid(format!("no order buffer for job {job_id}")))?;
        order
            .finished(claim.unit.idx, claim.unit.end_ms, (segments, gaps))
            .map_err(Error::from)?;
        self.flush_ready(job_id, now_ms)
    }

    /// Commits every unit of the job that is ready, in audio order, one at a time. A unit leaves the buffer only
    /// when its segments are in the JSONL (fsynced), its gaps are stored, the cursor has moved over it and its state
    /// says Committed; when any of that fails the unit goes back into the buffer, nothing after it is taken, the
    /// cursor stays and the error is returned, so the next call (the next unit to finish, or a retry) commits it first.
    /// A failed commit must never lose a unit or let the cursor pass it (ASR audit of 20 Nov 2026).
    pub fn flush_ready(&mut self, job_id: i64, now_ms: u64) -> Result<Completed> {
        let revision = match self.transcript.store().job(job_id)? {
            Some(j) => j.revision,
            None => return Ok(Completed::default()),
        };
        let mut out = Completed::default();
        while let Some(order) = self.orders.get_mut(&job_id) {
            let cursor_before = order.cursor_ms();
            let Some((idx, end_ms, (segs, gaps))) = order.pop_ready() else {
                break;
            };
            match self.commit_unit(job_id, &revision, idx, end_ms, &segs, &gaps) {
                Ok(refused) => {
                    out.refused += refused;
                    out.committed_units.push(idx);
                    out.segments.extend(segs);
                }
                Err(e) => {
                    if let Some(order) = self.orders.get_mut(&job_id) {
                        order.put_back(idx, end_ms, (segs, gaps), cursor_before);
                    }
                    return Err(e);
                }
            }
        }
        out.job_done = self.finalize_if_complete(job_id, now_ms)?;
        Ok(out)
    }

    /// One unit's commit; the segments are refused as repeats (counted) when the revision already holds them, and the
    /// gaps are replaced rather than added, so doing it again after a failure is safe.
    fn commit_unit(
        &mut self,
        job_id: i64,
        revision: &str,
        idx: u32,
        end_ms: u64,
        segs: &[Segment],
        gaps: &[Gap],
    ) -> Result<usize> {
        let committed = self.transcript.commit(revision, segs)?;
        for g in gaps {
            self.transcript.store_mut().add_gap(g)?;
        }
        self.transcript.store_mut().advance_cursor(job_id, end_ms)?;
        self.transcript
            .store_mut()
            .set_unit_state(job_id, idx, UnitState::Committed, None)?;
        Ok(committed.refused)
    }

    /// Marks the job (and its revision) complete when the plan is closed and every unit is committed. A
    /// complete `post-1` becomes the default; `post-2` replaces it as the default when it completes.
    fn finalize_if_complete(&mut self, job: i64, now_ms: u64) -> Result<bool> {
        let Some(j) = self.transcript.store().job(job)? else {
            return Ok(false);
        };
        if j.state == JobState::Done {
            return Ok(true);
        }
        // A night job copies its units whole; a rolling job is complete only after Stop closed the plan.
        let closed = self.plan.finished;
        let units = self.transcript.store().units(job)?;
        if !closed || units.is_empty() || units.iter().any(|u| u.state != UnitState::Committed) {
            return Ok(false);
        }
        self.transcript
            .store_mut()
            .set_job_state(job, JobState::Done, None, now_ms)?;
        self.transcript
            .store_mut()
            .mark_revision_complete(&j.revision)?;
        self.transcript
            .store_mut()
            .set_default_revision(&j.revision)?;
        self.orders.remove(&job);
        Ok(true)
    }

    /// Audio the job still has to decode, from the units not yet committed.
    pub fn remaining_audio_ms(&self, job: i64) -> Result<u64> {
        Ok(self
            .transcript
            .store()
            .units(job)?
            .iter()
            .filter(|u| u.state != UnitState::Committed)
            .map(|u| u.end_ms - u.start_ms)
            .sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use wb_protocol::SegmentFlag;

    fn temp(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "wb-runner-sq-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ev(unit: u32, chunk: u32, start: u64, end: u64, text: &str) -> SegmentEvent {
        SegmentEvent {
            unit,
            chunk,
            start_ms: start,
            end_ms: end,
            text: text.into(),
            lang: "hi".into(),
            lang_prob: 0.9,
            mixed: false,
            dir: "ltr".into(),
            avg_logprob: -0.3,
            no_speech_prob: 0.01,
            compression_ratio: 1.0,
            model: "m".into(),
            decoded_both_ways: false,
            detection: vec![],
            flags: vec![],
            truncated: false,
            hint_text: None,
        }
    }

    /// Writes what an engine would have staged for the unit and returns the claim's file.
    fn stage(claim: &Claim, events: &[SegmentEvent]) {
        stage_open(claim, events);
        let mut c = Committer::open_staged(&claim.staged).unwrap();
        c.commit_chunk(&[], claim.unit.end_ms).unwrap();
    }

    /// The same without the closing marker: an engine that was killed part way through the unit.
    fn stage_open(claim: &Claim, events: &[SegmentEvent]) {
        let mut c = Committer::open_staged(&claim.staged).unwrap();
        for e in events {
            c.commit_chunk(std::slice::from_ref(e), e.end_ms).unwrap();
        }
    }

    fn no_pauses() -> PartPauses {
        PartPauses::default()
    }

    /// A queue with one rolling job and `n` units of 10 s each (the cutter makes one unit for short audio, so
    /// the units are added directly).
    fn queue_with_units(name: &str, n: u32) -> (SessionQueue, i64, PathBuf) {
        let dir = temp(name);
        let mut q = SessionQueue::open(&dir).unwrap();
        let job = q
            .ensure_rolling_job(
                "hall-a",
                serde_json::json!({"hi": "m"}),
                vec!["en".into(), "hi".into()],
                1,
            )
            .unwrap();
        for i in 0..n {
            q.transcript
                .store_mut()
                .add_unit(
                    job,
                    i,
                    u64::from(i) * 10_000,
                    u64::from(i + 1) * 10_000,
                    None,
                )
                .unwrap();
        }
        (q, job, dir)
    }

    #[test]
    fn units_finishing_out_of_order_commit_in_audio_order_and_the_cursor_follows() {
        let (mut q, job, _dir) = queue_with_units("order", 3);
        let c0 = q.claim(Device::Cpu, 10).unwrap().unwrap();
        let c1 = q.claim(Device::Gpu, 11).unwrap().unwrap();
        assert_eq!((c0.unit.idx, c1.unit.idx), (0, 1));
        stage(&c1, &[ev(1, 0, 10_000, 14_000, "b")]);
        let done = q.complete(&c1, "t", 20).unwrap();
        // Unit 1 waits for unit 0: nothing is committed or shown, the cursor stays.
        assert!(done.committed_units.is_empty() && done.segments.is_empty());
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 0);
        stage(&c0, &[ev(0, 0, 500, 4_000, "a")]);
        let done = q.complete(&c0, "t", 21).unwrap();
        assert_eq!(done.committed_units, vec![0, 1]);
        let texts: Vec<_> = done.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b"]);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 20_000);
        assert_eq!(done.refused, 0);
        assert!(!done.job_done, "the plan is not closed yet");
        let _ = job;
    }

    #[test]
    fn a_commit_that_fails_loses_no_unit_and_commits_nothing_out_of_order() {
        let (mut q, _job, _dir) = queue_with_units("commit-fails", 3);
        let c0 = q.claim(Device::Cpu, 10).unwrap().unwrap();
        let c1 = q.claim(Device::Gpu, 11).unwrap().unwrap();
        stage(&c1, &[ev(1, 0, 10_000, 14_000, "b")]);
        q.complete(&c1, "t", 20).unwrap();
        stage(&c0, &[ev(0, 0, 500, 4_000, "a")]);
        q.transcript.inject_commit_failure_after(0);
        assert!(
            q.complete(&c0, "t", 21).is_err(),
            "the commit of unit 0 fails"
        );
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 0, "the cursor did not move");
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 0);
        // The next unit finishes: units 0, 1 and 2 are committed, in audio order, each once. Before the fix units 0
        // and 1 were gone from the buffer, unit 2 moved the cursor past them and they were never written.
        let c2 = q.claim(Device::Cpu, 22).unwrap().unwrap();
        assert_eq!(c2.unit.idx, 2);
        stage(&c2, &[ev(2, 0, 20_500, 24_000, "c")]);
        let done = q.complete(&c2, "t", 23).unwrap();
        assert_eq!(done.committed_units, vec![0, 1, 2]);
        let texts: Vec<_> = done.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 30_000);
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 3);
        assert_eq!(done.refused, 0);
    }

    #[test]
    fn a_resumed_job_gets_its_given_up_unit_back_with_fresh_attempts_and_finishes_it() {
        let (mut q, job, _dir) = queue_with_units("resume-retry", 1);
        // Three failures on the processor and no other device: the unit is given up and the job parked.
        let mut last = None;
        for t in 0..3u64 {
            let c = q.claim(Device::Cpu, 10 + t).unwrap().unwrap();
            last = Some(q.fail(&c, "engine died", false, 20 + t).unwrap());
        }
        assert_eq!(last, Some(Retry::GiveUp));
        assert_eq!(q.jobs().unwrap()[0].state, JobState::Parked);
        // The operator resumes. Before the fix the unit was Pending but its counts stayed at 3, nothing could claim
        // it, and the job stayed Queued for ever (which also kept the night pass waiting).
        q.set_paused(job, false, 30).unwrap();
        let c = q
            .claim(Device::Cpu, 31)
            .unwrap()
            .expect("the unit can be claimed again");
        assert_eq!(c.unit.idx, 0);
        stage(&c, &[ev(0, 0, 500, 4_000, "a")]);
        let done = q.complete(&c, "t", 32).unwrap();
        assert_eq!(done.committed_units, vec![0]);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 10_000);
    }

    #[test]
    fn a_unit_that_finishes_after_its_job_was_parked_commits_nothing_before_the_unit_before_it() {
        let (mut q, job, _dir) = queue_with_units("park-then-finish", 3);
        let c0 = q.claim(Device::Cpu, 10).unwrap().unwrap();
        let c1 = q.claim(Device::Gpu, 11).unwrap().unwrap();
        // Unit 0 is given up and the job is parked while the other engine is still decoding unit 1 (the normal
        // state of a hybrid). Before the fix unit 1's buffer started at unit 1: it was committed, the cursor passed
        // unit 0 and unit 0's text was refused as a repeat when it was decoded again.
        q.park_job(job, "unit 0 failed", 12).unwrap();
        stage(&c1, &[ev(1, 0, 10_000, 14_000, "b")]);
        let done = q.complete(&c1, "t", 13).unwrap();
        assert!(
            done.committed_units.is_empty() && done.segments.is_empty(),
            "unit 1 waits for unit 0"
        );
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 0, "the cursor did not move");
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 0);
        // The operator resumes and unit 0 is decoded: units 0 and 1 are committed in audio order, once each.
        q.set_paused(job, false, 14).unwrap();
        stage(&c0, &[ev(0, 0, 500, 4_000, "a")]);
        let done = q.complete(&c0, "t", 15).unwrap();
        assert_eq!(done.committed_units, vec![0, 1]);
        let texts: Vec<_> = done.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b"]);
        assert_eq!(done.refused, 0);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 20_000);
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 2);
    }

    #[test]
    fn resuming_a_parked_job_brings_back_the_units_decoded_before_the_park() {
        let (mut q, job, _dir) = queue_with_units("park-resume-decoded", 3);
        let c0 = q.claim(Device::Cpu, 10).unwrap().unwrap();
        let c1 = q.claim(Device::Gpu, 11).unwrap().unwrap();
        // Unit 1 is decoded and waits for unit 0; then the job is parked, which drops the buffer.
        stage(&c1, &[ev(1, 0, 10_000, 14_000, "b")]);
        q.complete(&c1, "t", 12).unwrap();
        q.park_job(job, "unit 0 failed", 13).unwrap();
        // Resume: the buffer is rebuilt with unit 1 in it (it is Decoded in the store; nothing would decode it
        // again). Before the fix the new buffer was empty, unit 1 was never committed and the job never finished.
        q.set_paused(job, false, 14).unwrap();
        stage(&c0, &[ev(0, 0, 500, 4_000, "a")]);
        let done = q.complete(&c0, "t", 15).unwrap();
        assert_eq!(done.committed_units, vec![0, 1]);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 20_000);
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 2);
    }

    #[test]
    fn a_failure_in_the_middle_keeps_the_units_after_it_and_a_retry_needs_no_new_unit() {
        let (mut q, job, _dir) = queue_with_units("commit-fails-mid", 3);
        let mut claims = Vec::new();
        for (i, d) in [Device::Cpu, Device::Gpu, Device::Cpu]
            .into_iter()
            .enumerate()
        {
            claims.push(q.claim(d, 10 + i as u64).unwrap().unwrap());
        }
        // Units 2 and 1 finish first and wait for unit 0.
        for idx in [2usize, 1] {
            let c = &claims[idx];
            stage(
                c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 100,
                    c.unit.end_ms - 100,
                    "x",
                )],
            );
            q.complete(c, "t", 20).unwrap();
        }
        // Unit 0 arrives: its commit goes through, then the commit of unit 1 fails.
        stage(&claims[0], &[ev(0, 0, 100, 9_000, "x")]);
        q.transcript.inject_commit_failure_after(1);
        assert!(q.complete(&claims[0], "t", 21).is_err());
        assert_eq!(
            q.jobs().unwrap()[0].cursor_ms,
            10_000,
            "only unit 0 is committed"
        );
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 1);
        // The retry commits units 1 and 2 in order, once each; the cursor ends at the end of unit 2.
        let done = q.flush_ready(job, 22).unwrap();
        assert_eq!(done.committed_units, vec![1, 2]);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 30_000);
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 3);
    }

    #[test]
    fn a_failed_commit_is_retried_by_flush_ready_without_another_unit() {
        let (mut q, job, _dir) = queue_with_units("commit-retry", 1);
        let c = q.claim(Device::Cpu, 10).unwrap().unwrap();
        stage(&c, &[ev(0, 0, 500, 4_000, "a")]);
        q.transcript.inject_commit_failure_after(0);
        assert!(q.complete(&c, "t", 11).is_err());
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 0);
        let done = q.flush_ready(job, 12).unwrap();
        assert_eq!(done.committed_units, vec![0]);
        assert_eq!(q.jobs().unwrap()[0].cursor_ms, 10_000);
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 1);
        // Nothing is left to commit: a second flush does nothing.
        assert!(q.flush_ready(job, 13).unwrap().committed_units.is_empty());
    }

    #[test]
    fn a_job_completes_when_the_plan_is_closed_and_every_unit_is_committed() {
        let (mut q, job, _dir) = queue_with_units("finish", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 100,
                    c.unit.end_ms - 100,
                    "x",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        assert_eq!(q.jobs().unwrap()[0].state, JobState::Running);
        // Stop: the plan closes (the audio is already covered by the two units, so no unit is added).
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        let j = q.jobs().unwrap().remove(0);
        assert_eq!(j.state, JobState::Done);
        assert!(q.transcript().store().revision_complete("post-1").unwrap());
        assert_eq!(
            q.transcript()
                .store()
                .default_revision()
                .unwrap()
                .as_deref(),
            Some("post-1")
        );
    }

    #[test]
    fn a_stopped_engine_leaves_its_unit_pending_and_the_staged_file_resumes_it() {
        let (mut q, _job, _dir) = queue_with_units("release", 1);
        let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
        stage(&c, &[ev(0, 0, 100, 3_000, "partial")]);
        q.release(&c).unwrap();
        let again = q.claim(Device::Cpu, 2).unwrap().unwrap();
        assert_eq!(again.unit.idx, 0);
        assert_eq!(again.staged, c.staged);
        let resumed = Committer::open_staged(&again.staged).unwrap();
        assert_eq!(resumed.segments(), 1);
    }

    #[test]
    fn three_failures_on_one_device_move_the_unit_to_the_other_then_the_job_parks() {
        let (mut q, job, _dir) = queue_with_units("fail", 1);
        for _ in 0..3 {
            let c = q.claim(Device::Gpu, 1).unwrap().unwrap();
            q.fail(&c, "device lost", true, 2).unwrap();
        }
        // The GPU has used its three tries: only the CPU may take it now.
        assert!(q.claim(Device::Gpu, 3).unwrap().is_none());
        for i in 0..3 {
            let c = q.claim(Device::Cpu, 4).unwrap().unwrap();
            let r = q.fail(&c, "out of memory", true, 5).unwrap();
            assert_eq!(r == Retry::GiveUp, i == 2, "{r:?}");
        }
        let j = q.transcript().store().job(job).unwrap().unwrap();
        assert_eq!(j.state, JobState::Parked);
        assert_eq!(j.error.as_deref(), Some("out of memory"));
        assert!(q.claim(Device::Cpu, 6).unwrap().is_none());
    }

    #[test]
    fn a_crash_between_decoding_and_committing_recommits_from_the_staged_files_without_a_repeat() {
        let dir = temp("crash");
        {
            let mut q = SessionQueue::open(&dir).unwrap();
            let job = q
                .ensure_rolling_job("s", serde_json::json!({}), vec!["hi".into()], 1)
                .unwrap();
            for i in 0..3u32 {
                q.transcript
                    .store_mut()
                    .add_unit(
                        job,
                        i,
                        u64::from(i) * 10_000,
                        u64::from(i + 1) * 10_000,
                        None,
                    )
                    .unwrap();
            }
            let c0 = q.claim(Device::Cpu, 1).unwrap().unwrap();
            let c1 = q.claim(Device::Cpu, 1).unwrap().unwrap();
            let c2 = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(&c0, &[ev(0, 0, 0, 5_000, "a")]);
            q.complete(&c0, "t", 2).unwrap();
            // Unit 1 decoded but unit 2 still running when the app dies; unit 1 is `decoded`, unit 2 `running`.
            stage(&c1, &[ev(1, 0, 10_000, 15_000, "b")]);
            q.transcript
                .store_mut()
                .set_unit_state(job, 1, UnitState::Decoded, Some(Device::Cpu))
                .unwrap();
            stage_open(&c2, &[ev(2, 0, 20_000, 22_000, "c")]);
        }
        let mut q = SessionQueue::open(&dir).unwrap();
        let units = q.units(1).unwrap();
        assert_eq!(units[0].state, UnitState::Committed);
        assert_eq!(units[1].state, UnitState::Decoded);
        assert_eq!(
            units[2].state,
            UnitState::Pending,
            "a running unit is pending again"
        );
        // Unit 2 is claimed again, finishes from its staged file, and commits behind unit 1.
        let c2 = q.claim(Device::Cpu, 5).unwrap().unwrap();
        assert_eq!(c2.unit.idx, 2);
        stage(&c2, &[ev(2, 1, 25_000, 28_000, "d")]);
        let done = q.complete(&c2, "t", 6).unwrap();
        assert_eq!(done.committed_units, vec![1, 2]);
        let all = q
            .transcript()
            .store()
            .segments("post-1", 0, u64::MAX)
            .unwrap();
        let texts: Vec<_> = all.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b", "c", "d"]);
        assert_eq!(done.refused, 0);
    }

    /// N6 (auditor of 20 Nov 2026): a segment reloaded from its staged file after a restart was committed with an empty
    /// `created_at`. The queue opened with a clock stamps it with the moment of the reload; segments completed in this run
    /// keep the stamp their completion was given.
    #[test]
    fn a_segment_reloaded_from_its_staged_file_gets_the_clocks_stamp_not_an_empty_created_at() {
        let dir = temp("crash-stamp");
        {
            let mut q = SessionQueue::open(&dir).unwrap();
            let job = q
                .ensure_rolling_job("s", serde_json::json!({}), vec!["hi".into()], 1)
                .unwrap();
            for i in 0..2u32 {
                q.transcript
                    .store_mut()
                    .add_unit(
                        job,
                        i,
                        u64::from(i) * 10_000,
                        u64::from(i + 1) * 10_000,
                        None,
                    )
                    .unwrap();
            }
            let c0 = q.claim(Device::Cpu, 1).unwrap().unwrap();
            let c1 = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(&c0, &[ev(0, 0, 0, 5_000, "a")]);
            q.complete(&c0, "run-1", 2).unwrap();
            stage(&c1, &[ev(1, 0, 10_000, 15_000, "b")]);
            q.transcript
                .store_mut()
                .set_unit_state(job, 1, UnitState::Decoded, Some(Device::Cpu))
                .unwrap();
        }
        let mut q = SessionQueue::open_with_clock(&dir, Arc::new(|| "reload".to_owned())).unwrap();
        let c1 = q.claim(Device::Cpu, 5).unwrap();
        assert!(c1.is_none(), "unit 1 is decoded and waits to be committed");
        // A third unit finishing commits the reloaded unit 1 behind it.
        let job = q.jobs().unwrap()[0].id;
        q.transcript
            .store_mut()
            .add_unit(job, 2, 20_000, 30_000, None)
            .unwrap();
        let c2 = q.claim(Device::Cpu, 6).unwrap().unwrap();
        stage(&c2, &[ev(2, 0, 20_000, 22_000, "c")]);
        q.complete(&c2, "run-2", 7).unwrap();
        let all = q
            .transcript()
            .store()
            .segments("post-1", 0, u64::MAX)
            .unwrap();
        let stamps: Vec<_> = all
            .iter()
            .map(|s| (s.text.as_str(), s.created_at.as_str()))
            .collect();
        assert_eq!(
            stamps,
            vec![("a", "run-1"), ("b", "reload"), ("c", "run-2")]
        );
    }

    #[test]
    fn a_crash_after_the_jsonl_but_before_the_state_does_not_repeat_the_unit() {
        let dir = temp("crash2");
        {
            let mut q = SessionQueue::open(&dir).unwrap();
            let job = q
                .ensure_rolling_job("s", serde_json::json!({}), vec!["hi".into()], 1)
                .unwrap();
            q.transcript
                .store_mut()
                .add_unit(job, 0, 0, 10_000, None)
                .unwrap();
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(&c, &[ev(0, 0, 100, 9_000, "once")]);
            // The segments reach the JSONL and SQLite and the cursor moves, then the power goes before the
            // unit is marked committed.
            let seg = to_segment(&ev(0, 0, 100, 9_000, "once"), "post-1", Device::Cpu, "t");
            q.transcript.commit("post-1", &[seg]).unwrap();
            q.transcript
                .store_mut()
                .advance_cursor(job, 10_000)
                .unwrap();
            q.transcript
                .store_mut()
                .set_unit_state(job, 0, UnitState::Decoded, Some(Device::Cpu))
                .unwrap();
        }
        let q = SessionQueue::open(&dir).unwrap();
        assert_eq!(q.units(1).unwrap()[0].state, UnitState::Committed);
        assert_eq!(
            q.transcript().store().segment_count("post-1").unwrap(),
            1,
            "committed once"
        );
    }

    #[test]
    fn silence_the_vad_called_non_speech_is_a_gap_but_a_flagged_segment_covers_its_range() {
        let mut unclear = to_segment(&ev(0, 1, 12_000, 18_000, ""), "post-1", Device::Cpu, "t");
        unclear.flags = vec!["unclear".into()];
        let segs = vec![
            to_segment(&ev(0, 0, 4_000, 6_000, "a"), "post-1", Device::Cpu, "t"),
            unclear,
        ];
        let gaps = vad_gaps("post-1", 0, 30_000, &segs);
        let ranges: Vec<_> = gaps.iter().map(|g| (g.start_ms, g.end_ms)).collect();
        // 0..4 s (4 s), 6..12 s (6 s), 18..30 s (12 s); the unclear chunk 12..18 s is not silence.
        assert_eq!(ranges, vec![(0, 4_000), (6_000, 12_000), (18_000, 30_000)]);
        assert!(gaps.iter().all(|g| g.kind == "vad_silence"));
        // Under two seconds is not a gap.
        assert!(
            vad_gaps("post-1", 0, 6_000, &segs[..1])
                .iter()
                .all(|g| g.end_ms - g.start_ms >= GAP_MIN_MS)
        );
    }

    #[test]
    fn engine_flags_become_the_transcripts_flag_names() {
        let mut e = ev(0, 0, 0, 1_000, "");
        e.flags = vec![SegmentFlag::Unclear, SegmentFlag::LowCoverage];
        let s = to_segment(&e, "post-1", Device::Gpu, "t");
        assert_eq!(s.flags, vec!["unclear", "low_coverage"]);
        assert_eq!(s.device, Device::Gpu);
    }

    #[test]
    fn the_rolling_plan_adds_units_as_parts_close_and_survives_a_restart() {
        let dir = temp("plan");
        let (job, n_first);
        {
            let mut q = SessionQueue::open(&dir).unwrap();
            job = q
                .ensure_rolling_job("s", serde_json::json!({}), vec!["hi".into()], 1)
                .unwrap();
            // Eight 10-minute parts with a pause every 20 s.
            for part in 0..8u32 {
                let start = u64::from(part) * 600_000;
                let pauses = PartPauses {
                    pauses: (0..30)
                        .map(|i| Pause {
                            start_ms: start + i * 20_000 + 9_000,
                            end_ms: start + i * 20_000 + 11_000,
                        })
                        .collect(),
                    fallback: vec![],
                };
                q.add_part(part, start, start + 600_000, 48_000, &pauses)
                    .unwrap();
                q.plan_units(job, false).unwrap();
            }
            n_first = q.units(job).unwrap().len();
            assert!(n_first >= 12, "{n_first} units for 80 minutes");
        }
        let mut q = SessionQueue::open(&dir).unwrap();
        assert_eq!(q.known_parts(), (0..8).collect::<Vec<_>>());
        assert_eq!(q.units(job).unwrap().len(), n_first);
        // Stop: the last unit reaches the end of the session and the units tile the audio.
        q.plan_units(job, true).unwrap();
        let units = q.units(job).unwrap();
        assert_eq!(units.last().unwrap().end_ms, 8 * 600_000);
        assert_eq!(units[0].start_ms, 0);
        for w in units.windows(2) {
            assert_eq!(w[0].end_ms, w[1].start_ms, "no gap, no overlap");
        }
    }

    #[test]
    fn a_night_job_copies_the_rolling_units_and_replaces_the_default_when_it_completes() {
        let (mut q, job, _dir) = queue_with_units("night", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "r",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        let night = q
            .create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 3)
            .unwrap();
        assert_eq!(q.units(night).unwrap().len(), 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 4).unwrap().unwrap();
            assert_eq!(c.job.id, night);
            assert_eq!(c.job.revision, "post-2");
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "n",
                )],
            );
            q.complete(&c, "t", 5).unwrap();
        }
        assert_eq!(
            q.transcript()
                .store()
                .default_revision()
                .unwrap()
                .as_deref(),
            Some("post-2")
        );
        // The rolling transcript is still there.
        assert_eq!(q.transcript().store().segment_count("post-1").unwrap(), 2);
    }

    #[test]
    fn a_night_job_held_for_a_new_session_keeps_its_units_and_goes_on_when_planned_again() {
        let (mut q, job, _dir) = queue_with_units("night-held", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "r",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        assert!(
            !q.hold_night_job(2).unwrap(),
            "no night job yet: nothing to hold"
        );
        let night = q
            .create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 3)
            .unwrap();
        // One night unit is committed, then another session starts: the planned night pass is cancelled.
        let c = q.claim(Device::Cpu, 4).unwrap().unwrap();
        stage(
            &c,
            &[ev(
                c.unit.idx,
                0,
                c.unit.start_ms + 10,
                c.unit.end_ms - 10,
                "n",
            )],
        );
        q.complete(&c, "t", 5).unwrap();
        assert!(q.hold_night_job(6).unwrap());
        assert!(
            q.claim(Device::Cpu, 7).unwrap().is_none(),
            "a held night job gives no unit while the new session records"
        );
        assert!(!q.hold_night_job(8).unwrap(), "already held");
        // Planned again after the last session: the same job (never a second one), resumed.
        let again = q
            .create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 9)
            .unwrap();
        assert_eq!(again, night);
        assert!(q.resume_night_job(10).unwrap());
        let c = q.claim(Device::Cpu, 11).unwrap().unwrap();
        assert_eq!(c.job.id, night);
        assert_eq!(c.unit.idx, 1, "the committed unit is not decoded again");
        stage(
            &c,
            &[ev(
                c.unit.idx,
                0,
                c.unit.start_ms + 10,
                c.unit.end_ms - 10,
                "n",
            )],
        );
        q.complete(&c, "t", 12).unwrap();
        assert_eq!(
            q.transcript()
                .store()
                .default_revision()
                .unwrap()
                .as_deref(),
            Some("post-2")
        );
        assert_eq!(q.transcript().store().segment_count("post-2").unwrap(), 2);
    }

    /// Item 3 of the auditor of 20 Nov 2026 (owner, 27 Nov 2026): the hold is the night job's own persisted state, so a
    /// hold survives a restart of the app and the pass is planned again for it; an operator's Pause is not a hold.
    #[test]
    fn a_night_hold_survives_an_app_restart_and_an_operators_pause_is_not_a_hold() {
        let (mut q, job, dir) = queue_with_units("night-hold-restart", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "r",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        let night = q
            .create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 3)
            .unwrap();
        assert!(!q.night_job_is_held().unwrap(), "not held yet");
        // The operator's own Pause is not a hold.
        q.set_paused(night, true, 4).unwrap();
        assert!(!q.night_job_is_held().unwrap(), "an operator's Pause");
        q.set_paused(night, false, 5).unwrap();
        // Another session starts: held. The app is closed and opened again.
        assert!(q.hold_night_job(6).unwrap());
        assert!(q.night_job_is_held().unwrap());
        drop(q);
        let mut q = SessionQueue::open(&dir).unwrap();
        assert!(
            q.night_job_is_held().unwrap(),
            "the hold is still there after a restart"
        );
        // Planned again after the last session: the same job goes on, and it is no longer held.
        assert!(q.resume_night_job(7).unwrap());
        assert!(!q.night_job_is_held().unwrap());
        let night_job = q
            .jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
            .unwrap();
        assert_eq!(night_job.state, JobState::Queued);
        assert!(night_job.error.is_none(), "the hold's note is gone");
    }

    /// Owner, 27 Nov 2026: a unit decoded without the Hinglish model after a graphics failure is flagged in the session
    /// folder, once per (job, unit), and the flag is still there after a restart.
    #[test]
    fn a_flagged_fallback_unit_is_written_once_and_survives_a_restart() {
        let (q, job, dir) = queue_with_units("fallback-flag", 2);
        assert!(q.fallback_units().is_empty());
        let models = serde_json::json!({"hi": "cpu-model"});
        q.flag_fallback_unit(job, 0, &models, 5).unwrap();
        q.flag_fallback_unit(job, 0, &models, 6).unwrap();
        q.flag_fallback_unit(job, 1, &models, 7).unwrap();
        assert_eq!(q.fallback_units(), vec![(job, 0), (job, 1)]);
        drop(q);
        let q = SessionQueue::open(&dir).unwrap();
        assert_eq!(q.fallback_units(), vec![(job, 0), (job, 1)]);
        assert!(dir.join(FALLBACK_UNITS_FILE).exists());
    }

    #[test]
    fn a_held_night_job_that_the_new_plan_does_not_run_is_parked_with_the_reason() {
        let (mut q, job, _dir) = queue_with_units("night-parked", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "r",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        assert!(
            !q.park_held_night_job("x", 3).unwrap(),
            "no night job yet: nothing to park"
        );
        q.create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 3)
            .unwrap();
        assert!(
            !q.park_held_night_job("x", 4).unwrap(),
            "a night job that is not held is left alone"
        );
        assert!(q.hold_night_job(5).unwrap());
        assert!(
            q.park_held_night_job("night pass skipped: it does not fit", 6)
                .unwrap()
        );
        let night = q
            .jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
            .unwrap();
        assert_eq!(night.state, JobState::Parked);
        assert!(
            q.claim(Device::Cpu, 7).unwrap().is_none(),
            "a parked night job gives no unit"
        );
        // The rolling transcript stays the default revision.
        assert_eq!(
            q.transcript()
                .store()
                .default_revision()
                .unwrap()
                .as_deref(),
            Some("post-1")
        );
    }

    /// Owner, 27 Nov 2026: the night safety switch drops a night job that has not started and never one that has.
    #[test]
    fn the_night_safety_switch_drops_a_night_job_that_has_not_started_and_never_one_that_has() {
        let (mut q, job, _dir) = queue_with_units("night-dropped", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "r",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        assert!(
            !q.drop_unstarted_night_job("x", 3).unwrap(),
            "no night job yet: nothing to drop"
        );
        q.create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 3)
            .unwrap();
        // A unit of the night job is claimed (running): the job has started and stays.
        let running = q.claim(Device::Cpu, 4).unwrap().unwrap();
        assert!(
            !q.drop_unstarted_night_job("x", 5).unwrap(),
            "a job with a running unit is never stopped"
        );
        q.release(&running).unwrap();
        // Nothing has started now: the job is dropped with the reason and gives no unit.
        assert!(
            q.drop_unstarted_night_job("the night ran slower than planned", 6)
                .unwrap()
        );
        let night = q
            .jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.kind == JobKind::Night)
            .unwrap();
        assert_eq!(night.state, JobState::Parked);
        assert_eq!(
            night.error.as_deref(),
            Some("the night ran slower than planned")
        );
        assert!(q.claim(Device::Cpu, 7).unwrap().is_none());
        assert_eq!(
            q.transcript()
                .store()
                .default_revision()
                .unwrap()
                .as_deref(),
            Some("post-1"),
            "the rolling transcript stands"
        );
    }

    /// Auditor, second run: the models of a night job change only while no unit of it has started, in one statement.
    #[test]
    fn the_models_of_a_night_job_change_only_before_a_unit_has_started() {
        let (mut q, job, _dir) = queue_with_units("night-models", 2);
        for _ in 0..2 {
            let c = q.claim(Device::Cpu, 1).unwrap().unwrap();
            stage(
                &c,
                &[ev(
                    c.unit.idx,
                    0,
                    c.unit.start_ms + 10,
                    c.unit.end_ms - 10,
                    "r",
                )],
            );
            q.complete(&c, "t", 2).unwrap();
        }
        q.add_part(1, 0, 20_000, 48_000, &no_pauses()).unwrap();
        q.plan_units(job, true).unwrap();
        let night = q
            .create_night_job("s", serde_json::json!({"hi": "big"}), vec!["hi".into()], 3)
            .unwrap();
        // A job created after the night job does not sort among the night ranks.
        assert!(q.rank_night_job(2).unwrap());
        let later = q
            .transcript()
            .store()
            .jobs()
            .unwrap()
            .into_iter()
            .map(|j| (j.kind, j.position))
            .collect::<Vec<_>>();
        assert!(
            later
                .iter()
                .any(|(k, p)| *k == JobKind::Night && *p == NIGHT_RANK_BASE + 2),
            "{later:?}"
        );
        assert!(
            q.change_models_if_unstarted(night, &serde_json::json!({"hi": "other"}))
                .unwrap()
        );
        let claim = q.claim(Device::Cpu, 4).unwrap().unwrap();
        assert_eq!(claim.job.id, night);
        assert!(
            !q.change_models_if_unstarted(night, &serde_json::json!({"hi": "third"}))
                .unwrap(),
            "a job with a running unit keeps its models"
        );
        q.release(&claim).unwrap();
    }

    #[test]
    fn a_night_job_cannot_start_before_the_rolling_pass_is_complete() {
        let (mut q, _job, _dir) = queue_with_units("night-early", 1);
        assert!(
            q.create_night_job("s", serde_json::json!({}), vec![], 1)
                .is_err()
        );
    }

    #[test]
    fn rolling_beats_night_and_the_operator_pause_holds_a_job() {
        let (mut q, job, _dir) = queue_with_units("pause", 1);
        q.set_paused(job, true, 1).unwrap();
        assert!(q.claim(Device::Cpu, 2).unwrap().is_none());
        q.set_paused(job, false, 3).unwrap();
        assert!(q.claim(Device::Cpu, 4).unwrap().is_some());
    }
}
