//! The session's `transcript.db`: jobs, work units, segments, revisions and gaps (PLAN section 8, Phase 4
//! step 1). One file per session, so it moves with the folder for import and archive.
//!
//! SQLite is the index, the JSONL files are the truth ([`crate::revision`]): a segment is in a revision's
//! JSONL (written and fsynced) before it is in this database, and [`crate::revision::Transcript::open`]
//! rebuilds any row that is behind.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Rolling,
    Night,
    Translate,
    Import,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Rolling => "rolling",
            JobKind::Night => "night",
            JobKind::Translate => "translate",
            JobKind::Import => "import",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "rolling" => JobKind::Rolling,
            "night" => JobKind::Night,
            "translate" => JobKind::Translate,
            "import" => JobKind::Import,
            other => return Err(Error::Invalid(format!("unknown job kind '{other}'"))),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting for audio or for a free engine.
    Queued,
    Running,
    /// Held by the scheduler (the reason is in the scheduler log) or by the operator.
    Paused,
    Done,
    /// A unit failed on every device; the error is shown on the Queue row.
    Parked,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Done => "done",
            JobState::Parked => "parked",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "queued" => JobState::Queued,
            "running" => JobState::Running,
            "paused" => JobState::Paused,
            "done" => JobState::Done,
            "parked" => JobState::Parked,
            other => return Err(Error::Invalid(format!("unknown job state '{other}'"))),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitState {
    Pending,
    Running,
    /// Decoded and waiting in the in-order buffer; not yet committed.
    Decoded,
    Committed,
    /// Failed on every device.
    Failed,
}

impl UnitState {
    pub fn as_str(self) -> &'static str {
        match self {
            UnitState::Pending => "pending",
            UnitState::Running => "running",
            UnitState::Decoded => "decoded",
            UnitState::Committed => "committed",
            UnitState::Failed => "failed",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "pending" => UnitState::Pending,
            "running" => UnitState::Running,
            "decoded" => UnitState::Decoded,
            "committed" => UnitState::Committed,
            "failed" => UnitState::Failed,
            other => return Err(Error::Invalid(format!("unknown unit state '{other}'"))),
        })
    }
}

/// Where a unit runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Cpu,
    Gpu,
}

impl Device {
    pub fn as_str(self) -> &'static str {
        match self {
            Device::Cpu => "cpu",
            Device::Gpu => "gpu",
        }
    }
    pub fn other(self) -> Device {
        match self {
            Device::Cpu => Device::Gpu,
            Device::Gpu => Device::Cpu,
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "cpu" => Device::Cpu,
            "gpu" => Device::Gpu,
            other => return Err(Error::Invalid(format!("unknown device '{other}'"))),
        })
    }
}

/// A unit that fails this many times on one device moves to the other device (Phase 4 step 7).
pub const ATTEMPTS_PER_DEVICE: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    pub session: String,
    pub kind: JobKind,
    /// `{"hi": "<model id>", "*": "<model id>"}` as the protocol's `models_by_lang`.
    pub models_by_lang: serde_json::Value,
    pub languages: Vec<String>,
    /// `post-1` for rolling, `post-2` for night.
    pub revision: String,
    pub created_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: i64,
    pub session: String,
    pub kind: JobKind,
    pub models_by_lang: serde_json::Value,
    pub languages: Vec<String>,
    pub revision: String,
    pub state: JobState,
    /// Every unit up to here is committed (`committed_until_ms`).
    pub cursor_ms: u64,
    /// Queue order: lower runs first within a kind. Move Up swaps it with the job before.
    pub position: i64,
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub id: i64,
    pub job: i64,
    pub idx: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub state: UnitState,
    /// The device that is running or last ran the unit.
    pub device: Option<Device>,
    pub cpu_attempts: u32,
    pub gpu_attempts: u32,
    pub last_error: Option<String>,
    /// Why the cut is not at a VAD pause (`forced_cut`, `rate_change`), if it is not.
    pub seam: Option<String>,
}

impl Unit {
    pub fn attempts(&self, d: Device) -> u32 {
        match d {
            Device::Cpu => self.cpu_attempts,
            Device::Gpu => self.gpu_attempts,
        }
    }
}

/// One committed segment of a revision (PLAN section 8 fields). The same shape is a line of the JSONL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub revision: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub lang: String,
    pub lang_prob: f32,
    pub mixed: bool,
    pub dir: String,
    pub avg_logprob: f32,
    pub no_speech_prob: f32,
    pub model: String,
    pub device: Device,
    #[serde(default)]
    pub flags: Vec<String>,
    /// Review only; never part of the transcript (`wb_protocol::SegmentEvent::hint_text`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_text: Option<String>,
    /// IST wall-clock text, for the UI only.
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub revision: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// `vad_silence` for audio the VAD called non-speech.
    pub kind: String,
}

pub struct Store {
    conn: Connection,
}

fn ms(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn db_ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl Store {
    /// Opens (and creates) the session's database in WAL mode.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "transcript.db has schema {version}, this app knows {SCHEMA_VERSION}"
            )));
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY,
                session TEXT NOT NULL,
                kind TEXT NOT NULL,
                models_by_lang TEXT NOT NULL,
                languages TEXT NOT NULL,
                revision TEXT NOT NULL,
                state TEXT NOT NULL,
                cursor_ms INTEGER NOT NULL DEFAULT 0,
                position INTEGER NOT NULL,
                created_ms INTEGER NOT NULL,
                started_ms INTEGER,
                finished_ms INTEGER,
                error TEXT
             );
             CREATE TABLE IF NOT EXISTS units (
                id INTEGER PRIMARY KEY,
                job INTEGER NOT NULL REFERENCES jobs(id),
                idx INTEGER NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                state TEXT NOT NULL,
                device TEXT,
                cpu_attempts INTEGER NOT NULL DEFAULT 0,
                gpu_attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                seam TEXT,
                UNIQUE (job, idx)
             );
             CREATE TABLE IF NOT EXISTS revisions (
                name TEXT PRIMARY KEY,
                complete INTEGER NOT NULL DEFAULT 0,
                is_default INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS segments (
                revision TEXT NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                lang TEXT NOT NULL,
                lang_prob REAL NOT NULL,
                mixed INTEGER NOT NULL,
                device TEXT NOT NULL,
                body TEXT NOT NULL,
                PRIMARY KEY (revision, start_ms)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS gaps (
                revision TEXT NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                kind TEXT NOT NULL,
                PRIMARY KEY (revision, start_ms)
             ) WITHOUT ROWID;",
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Store { conn })
    }

    // ---- jobs ----

    pub fn create_job(&mut self, new: &NewJob) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let position: i64 =
            tx.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM jobs", [], |r| {
                r.get(0)
            })?;
        tx.execute(
            "INSERT INTO jobs (session, kind, models_by_lang, languages, revision, state, position, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7)",
            params![
                new.session,
                new.kind.as_str(),
                serde_json::to_string(&new.models_by_lang)?,
                serde_json::to_string(&new.languages)?,
                new.revision,
                position,
                db_ms(new.created_ms),
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT OR IGNORE INTO revisions (name) VALUES (?1)",
            params![new.revision],
        )?;
        tx.commit()?;
        Ok(id)
    }

    fn job_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Job, String, String, String)> {
        let kind: String = r.get("kind")?;
        let state: String = r.get("state")?;
        let models: String = r.get("models_by_lang")?;
        let languages: String = r.get("languages")?;
        let job = Job {
            id: r.get("id")?,
            session: r.get("session")?,
            kind: JobKind::Rolling,
            models_by_lang: serde_json::Value::Null,
            languages: Vec::new(),
            revision: r.get("revision")?,
            state: JobState::Queued,
            cursor_ms: ms(r.get("cursor_ms")?),
            position: r.get("position")?,
            created_ms: ms(r.get("created_ms")?),
            started_ms: r.get::<_, Option<i64>>("started_ms")?.map(ms),
            finished_ms: r.get::<_, Option<i64>>("finished_ms")?.map(ms),
            error: r.get("error")?,
        };
        Ok((job, kind + "\u{0}" + &state, models, languages))
    }

    fn finish_job(parts: (Job, String, String, String)) -> Result<Job> {
        let (mut job, ks, models, languages) = parts;
        let (kind, state) = ks.split_once('\u{0}').unwrap_or((&ks, "queued"));
        job.kind = JobKind::parse(kind)?;
        job.state = JobState::parse(state)?;
        job.models_by_lang = serde_json::from_str(&models)?;
        job.languages = serde_json::from_str(&languages)?;
        Ok(job)
    }

    pub fn job(&self, id: i64) -> Result<Option<Job>> {
        let row = self
            .conn
            .query_row(
                "SELECT * FROM jobs WHERE id = ?1",
                params![id],
                Self::job_from_row,
            )
            .optional()?;
        row.map(Self::finish_job).transpose()
    }

    /// Every job in queue order (position).
    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut st = self.conn.prepare("SELECT * FROM jobs ORDER BY position")?;
        let rows = st.query_map([], Self::job_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(Self::finish_job(r?)?);
        }
        Ok(out)
    }

    pub fn set_job_state(
        &mut self,
        id: i64,
        state: JobState,
        error: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let started = matches!(state, JobState::Running);
        let finished = matches!(state, JobState::Done | JobState::Parked);
        self.conn.execute(
            "UPDATE jobs SET state = ?2, error = ?3,
                 started_ms = CASE WHEN ?4 AND started_ms IS NULL THEN ?6 ELSE started_ms END,
                 finished_ms = CASE WHEN ?5 THEN ?6 ELSE finished_ms END
             WHERE id = ?1",
            params![id, state.as_str(), error, started, finished, db_ms(now_ms)],
        )?;
        Ok(())
    }

    /// The cursor only moves forward: a lower value is ignored (a restarted engine never moves it back).
    pub fn advance_cursor(&mut self, id: i64, cursor_ms: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET cursor_ms = MAX(cursor_ms, ?2) WHERE id = ?1",
            params![id, db_ms(cursor_ms)],
        )?;
        Ok(())
    }

    /// Move Up: swaps the job's place with the job before it of the same kind. Returns whether it moved.
    pub fn move_up(&mut self, id: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let me: Option<(i64, String)> = tx
            .query_row(
                "SELECT position, kind FROM jobs WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((pos, kind)) = me else {
            return Ok(false);
        };
        let before: Option<(i64, i64)> = tx
            .query_row(
                "SELECT id, position FROM jobs WHERE kind = ?1 AND position < ?2 AND state != 'done'
                 ORDER BY position DESC LIMIT 1",
                params![kind, pos],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((other, other_pos)) = before else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE jobs SET position = ?2 WHERE id = ?1",
            params![id, other_pos],
        )?;
        tx.execute(
            "UPDATE jobs SET position = ?2 WHERE id = ?1",
            params![other, pos],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Change Model: the models used by the units that have not started (running units finish as they are).
    pub fn set_models(&mut self, id: i64, models_by_lang: &serde_json::Value) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET models_by_lang = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(models_by_lang)?],
        )?;
        Ok(())
    }

    // ---- units ----

    /// Adds a unit. Adding the same (job, idx) again with the same range is a no-op (a rolling job is
    /// extended on each closed part and on a restart); a different range for an existing index is an error.
    pub fn add_unit(
        &mut self,
        job: i64,
        idx: u32,
        start_ms: u64,
        end_ms: u64,
        seam: Option<&str>,
    ) -> Result<()> {
        if end_ms <= start_ms {
            return Err(Error::Invalid(format!(
                "unit {idx} of job {job} is empty ({start_ms}..{end_ms})"
            )));
        }
        let existing: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT start_ms, end_ms FROM units WHERE job = ?1 AND idx = ?2",
                params![job, idx],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((s, e)) if ms(s) == start_ms && ms(e) == end_ms => Ok(()),
            Some(_) => Err(Error::Invalid(format!(
                "unit {idx} of job {job} already exists with another range"
            ))),
            None => {
                self.conn.execute(
                    "INSERT INTO units (job, idx, start_ms, end_ms, state, seam)
                     VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
                    params![job, idx, db_ms(start_ms), db_ms(end_ms), seam],
                )?;
                Ok(())
            }
        }
    }

    fn unit_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Unit, String, Option<String>)> {
        let state: String = r.get("state")?;
        let device: Option<String> = r.get("device")?;
        let unit = Unit {
            id: r.get("id")?,
            job: r.get("job")?,
            idx: r.get("idx")?,
            start_ms: ms(r.get("start_ms")?),
            end_ms: ms(r.get("end_ms")?),
            state: UnitState::Pending,
            device: None,
            cpu_attempts: r.get("cpu_attempts")?,
            gpu_attempts: r.get("gpu_attempts")?,
            last_error: r.get("last_error")?,
            seam: r.get("seam")?,
        };
        Ok((unit, state, device))
    }

    fn finish_unit(parts: (Unit, String, Option<String>)) -> Result<Unit> {
        let (mut unit, state, device) = parts;
        unit.state = UnitState::parse(&state)?;
        unit.device = device.as_deref().map(Device::parse).transpose()?;
        Ok(unit)
    }

    pub fn units(&self, job: i64) -> Result<Vec<Unit>> {
        let mut st = self
            .conn
            .prepare("SELECT * FROM units WHERE job = ?1 ORDER BY idx")?;
        let rows = st.query_map(params![job], Self::unit_from_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(Self::finish_unit(r?)?);
        }
        Ok(out)
    }

    pub fn set_unit_state(
        &mut self,
        job: i64,
        idx: u32,
        state: UnitState,
        device: Option<Device>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE units SET state = ?3, device = COALESCE(?4, device) WHERE job = ?1 AND idx = ?2",
            params![job, idx, state.as_str(), device.map(Device::as_str)],
        )?;
        Ok(())
    }

    /// Counts a failed attempt on `device` and returns what to do next: retry on the same device, move to the
    /// other device after [`ATTEMPTS_PER_DEVICE`] failures, or give up when both devices have failed that
    /// many times (the job is then parked). A single-device machine passes `other_device_available = false`.
    pub fn record_failure(
        &mut self,
        job: i64,
        idx: u32,
        device: Device,
        error: &str,
        other_device_available: bool,
    ) -> Result<Retry> {
        let col = match device {
            Device::Cpu => "cpu_attempts",
            Device::Gpu => "gpu_attempts",
        };
        self.conn.execute(
            &format!(
                "UPDATE units SET {col} = {col} + 1, last_error = ?3, state = 'pending' WHERE job = ?1 AND idx = ?2"
            ),
            params![job, idx, error],
        )?;
        let unit = self
            .units(job)?
            .into_iter()
            .find(|u| u.idx == idx)
            .ok_or_else(|| Error::Invalid(format!("no unit {idx} in job {job}")))?;
        let this = unit.attempts(device);
        let other = unit.attempts(device.other());
        let retry = if this < ATTEMPTS_PER_DEVICE {
            Retry::Same(device)
        } else if other_device_available && other < ATTEMPTS_PER_DEVICE {
            Retry::Other(device.other())
        } else {
            Retry::GiveUp
        };
        if retry == Retry::GiveUp {
            self.set_unit_state(job, idx, UnitState::Failed, None)?;
        }
        Ok(retry)
    }

    // ---- revisions, segments, gaps ----

    pub fn ensure_revision(&mut self, name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO revisions (name) VALUES (?1)",
            params![name],
        )?;
        Ok(())
    }

    pub fn mark_revision_complete(&mut self, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE revisions SET complete = 1 WHERE name = ?1",
            params![name],
        )?;
        Ok(())
    }

    pub fn revision_complete(&self, name: &str) -> Result<bool> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT complete FROM revisions WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v == Some(1))
    }

    /// Makes `name` the default revision; only a complete revision may become the default.
    pub fn set_default_revision(&mut self, name: &str) -> Result<()> {
        if !self.revision_complete(name)? {
            return Err(Error::Invalid(format!("revision {name} is not complete")));
        }
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE revisions SET is_default = 0", [])?;
        tx.execute(
            "UPDATE revisions SET is_default = 1 WHERE name = ?1",
            params![name],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn default_revision(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT name FROM revisions WHERE is_default = 1", [], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Inserts segments in one transaction. A segment whose (revision, start_ms) is already there is skipped,
    /// which is what makes a replay after a crash harmless. Returns how many were new.
    pub fn insert_segments(&mut self, segments: &[Segment]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut added = 0;
        {
            let mut st = tx.prepare(
                "INSERT OR IGNORE INTO segments (revision, start_ms, end_ms, lang, lang_prob, mixed, device, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for s in segments {
                added += st.execute(params![
                    s.revision,
                    db_ms(s.start_ms),
                    db_ms(s.end_ms),
                    s.lang,
                    f64::from(s.lang_prob),
                    s.mixed,
                    s.device.as_str(),
                    serde_json::to_string(s)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// The segments of a revision overlapping `[from_ms, to_ms)`, in audio order.
    pub fn segments(&self, revision: &str, from_ms: u64, to_ms: u64) -> Result<Vec<Segment>> {
        let mut st = self.conn.prepare(
            "SELECT body FROM segments WHERE revision = ?1 AND end_ms > ?2 AND start_ms < ?3 ORDER BY start_ms",
        )?;
        let rows = st.query_map(params![revision, db_ms(from_ms), db_ms(to_ms)], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    pub fn segment_count(&self, revision: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM segments WHERE revision = ?1",
            params![revision],
            |r| r.get(0),
        )?;
        Ok(ms(n))
    }

    pub fn add_gap(&mut self, gap: &Gap) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO gaps (revision, start_ms, end_ms, kind) VALUES (?1, ?2, ?3, ?4)",
            params![gap.revision, db_ms(gap.start_ms), db_ms(gap.end_ms), gap.kind],
        )?;
        Ok(())
    }

    pub fn gaps(&self, revision: &str) -> Result<Vec<Gap>> {
        let mut st = self
            .conn
            .prepare("SELECT revision, start_ms, end_ms, kind FROM gaps WHERE revision = ?1 ORDER BY start_ms")?;
        let rows = st.query_map(params![revision], |r| {
            Ok(Gap {
                revision: r.get(0)?,
                start_ms: ms(r.get(1)?),
                end_ms: ms(r.get(2)?),
                kind: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

/// What to do with a unit after a failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    Same(Device),
    Other(Device),
    GiveUp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::seg;

    fn new_job(store: &mut Store, kind: JobKind) -> i64 {
        store
            .create_job(&NewJob {
                session: "hall-a".into(),
                kind,
                models_by_lang: serde_json::json!({"hi": "m1"}),
                languages: vec!["en".into(), "hi".into()],
                revision: if kind == JobKind::Night {
                    "post-2"
                } else {
                    "post-1"
                }
                .into(),
                created_ms: 1_000,
            })
            .unwrap()
    }

    #[test]
    fn a_job_round_trips_with_its_models_and_languages() {
        let mut s = Store::open_in_memory().unwrap();
        let id = new_job(&mut s, JobKind::Rolling);
        let j = s.job(id).unwrap().unwrap();
        assert_eq!(j.kind, JobKind::Rolling);
        assert_eq!(j.state, JobState::Queued);
        assert_eq!(j.languages, vec!["en", "hi"]);
        assert_eq!(j.models_by_lang["hi"], "m1");
        assert_eq!(j.revision, "post-1");
    }

    #[test]
    fn the_cursor_never_moves_back() {
        let mut s = Store::open_in_memory().unwrap();
        let id = new_job(&mut s, JobKind::Rolling);
        s.advance_cursor(id, 5_000).unwrap();
        s.advance_cursor(id, 3_000).unwrap();
        assert_eq!(s.job(id).unwrap().unwrap().cursor_ms, 5_000);
    }

    #[test]
    fn only_a_complete_revision_becomes_the_default() {
        let mut s = Store::open_in_memory().unwrap();
        new_job(&mut s, JobKind::Night);
        assert!(s.set_default_revision("post-2").is_err());
        s.mark_revision_complete("post-2").unwrap();
        s.set_default_revision("post-2").unwrap();
        assert_eq!(s.default_revision().unwrap().as_deref(), Some("post-2"));
    }

    #[test]
    fn move_up_swaps_with_the_job_before_of_the_same_kind() {
        let mut s = Store::open_in_memory().unwrap();
        let a = new_job(&mut s, JobKind::Rolling);
        let n = new_job(&mut s, JobKind::Night);
        let b = new_job(&mut s, JobKind::Rolling);
        assert!(s.move_up(b).unwrap());
        let order: Vec<i64> = s.jobs().unwrap().iter().map(|j| j.id).collect();
        assert_eq!(order, vec![b, n, a]);
        assert!(!s.move_up(b).unwrap(), "already first of its kind");
    }

    #[test]
    fn a_unit_added_twice_with_the_same_range_is_one_unit_and_a_different_range_is_refused() {
        let mut s = Store::open_in_memory().unwrap();
        let id = new_job(&mut s, JobKind::Rolling);
        s.add_unit(id, 0, 0, 300_000, None).unwrap();
        s.add_unit(id, 0, 0, 300_000, None).unwrap();
        assert!(s.add_unit(id, 0, 0, 290_000, None).is_err());
        assert!(s.add_unit(id, 1, 5, 5, None).is_err());
        assert_eq!(s.units(id).unwrap().len(), 1);
    }

    #[test]
    fn three_failures_on_a_device_move_the_unit_to_the_other_device_and_then_park_it() {
        let mut s = Store::open_in_memory().unwrap();
        let id = new_job(&mut s, JobKind::Rolling);
        s.add_unit(id, 0, 0, 1_000, None).unwrap();
        for _ in 0..2 {
            assert_eq!(
                s.record_failure(id, 0, Device::Gpu, "boom", true).unwrap(),
                Retry::Same(Device::Gpu)
            );
        }
        assert_eq!(
            s.record_failure(id, 0, Device::Gpu, "boom", true).unwrap(),
            Retry::Other(Device::Cpu)
        );
        for _ in 0..2 {
            assert_eq!(
                s.record_failure(id, 0, Device::Cpu, "boom", true).unwrap(),
                Retry::Same(Device::Cpu)
            );
        }
        assert_eq!(
            s.record_failure(id, 0, Device::Cpu, "boom", true).unwrap(),
            Retry::GiveUp
        );
        assert_eq!(s.units(id).unwrap()[0].state, UnitState::Failed);
    }

    #[test]
    fn a_single_device_machine_gives_up_after_three_failures() {
        let mut s = Store::open_in_memory().unwrap();
        let id = new_job(&mut s, JobKind::Rolling);
        s.add_unit(id, 0, 0, 1_000, None).unwrap();
        s.record_failure(id, 0, Device::Cpu, "x", false).unwrap();
        s.record_failure(id, 0, Device::Cpu, "x", false).unwrap();
        assert_eq!(
            s.record_failure(id, 0, Device::Cpu, "x", false).unwrap(),
            Retry::GiveUp
        );
    }

    #[test]
    fn a_segment_inserted_twice_is_stored_once() {
        let mut s = Store::open_in_memory().unwrap();
        let segs = vec![seg("post-1", 0, 900, "a"), seg("post-1", 1_000, 1_900, "b")];
        assert_eq!(s.insert_segments(&segs).unwrap(), 2);
        assert_eq!(s.insert_segments(&segs).unwrap(), 0);
        assert_eq!(s.segment_count("post-1").unwrap(), 2);
        let got = s.segments("post-1", 500, 1_200).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "a");
    }

    #[test]
    fn the_database_survives_a_reopen_on_disk() {
        let dir = crate::testutil::temp("store-reopen");
        let path = dir.join("transcript.db");
        {
            let mut s = Store::open(&path).unwrap();
            let id = new_job(&mut s, JobKind::Rolling);
            s.insert_segments(&[seg("post-1", 0, 100, "x")]).unwrap();
            s.advance_cursor(id, 100).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.jobs().unwrap()[0].cursor_ms, 100);
        assert_eq!(s.segment_count("post-1").unwrap(), 1);
    }
}
