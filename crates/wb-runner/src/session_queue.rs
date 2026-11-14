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

pub struct SessionQueue {
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
    /// Opens the session's queue, recovering from a crash.
    pub fn open(dir: &Path) -> Result<Self> {
        let transcript = Transcript::open(dir)?;
        let plan = match std::fs::read(plan_path(dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Invalid(format!("{}: {e}", plan_path(dir).display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PlanState::default(),
            Err(e) => return Err(io(&plan_path(dir))(e)),
        };
        let mut q = Self {
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
                    UnitState::Running => {
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
                                    .map(|e| to_segment(e, &job.revision, device, ""))
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
            if matches!(job.state, JobState::Queued | JobState::Running) {
                self.orders.insert(job.id, order);
            }
        }
        Ok(())
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
            if next == JobState::Queued && !self.orders.contains_key(&job) {
                self.reopen_order(job)?;
            }
        }
        Ok(())
    }

    fn reopen_order(&mut self, job: i64) -> Result<()> {
        let j = self
            .transcript
            .store()
            .job(job)?
            .ok_or_else(|| Error::Invalid(format!("no job {job}")))?;
        let units = self.transcript.store().units(job)?;
        let next = units
            .iter()
            .find(|u| u.state != UnitState::Committed)
            .map_or(units.iter().map(|u| u.idx + 1).max().unwrap_or(0), |u| {
                u.idx
            });
        self.orders.insert(job, OrderBuffer::new(next, j.cursor_ms));
        // A parked unit gets another try (the operator asked to resume).
        for u in units.iter().filter(|u| u.state == UnitState::Failed) {
            self.transcript
                .store_mut()
                .set_unit_state(job, u.idx, UnitState::Pending, None)?;
        }
        Ok(())
    }

    pub fn move_up(&mut self, job: i64) -> Result<bool> {
        Ok(self.transcript.store_mut().move_up(job)?)
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
        self.transcript.store_mut().set_unit_state(
            claim.job.id,
            claim.unit.idx,
            UnitState::Decoded,
            Some(claim.device),
        )?;
        let job_id = claim.job.id;
        let order = self
            .orders
            .entry(job_id)
            .or_insert_with(|| OrderBuffer::new(claim.unit.idx, claim.job.cursor_ms));
        order
            .finished(claim.unit.idx, claim.unit.end_ms, (segments, gaps))
            .map_err(Error::from)?;
        let ready = order.drain_ready();
        let mut out = Completed::default();
        let revision = claim.job.revision.clone();
        for (idx, end_ms, (segs, gaps)) in ready {
            let committed = self.transcript.commit(&revision, &segs)?;
            out.refused += committed.refused;
            for g in &gaps {
                self.transcript.store_mut().add_gap(g)?;
            }
            self.transcript.store_mut().advance_cursor(job_id, end_ms)?;
            self.transcript
                .store_mut()
                .set_unit_state(job_id, idx, UnitState::Committed, None)?;
            out.committed_units.push(idx);
            out.segments.extend(segs);
        }
        out.job_done = self.finalize_if_complete(job_id, now_ms)?;
        Ok(out)
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
