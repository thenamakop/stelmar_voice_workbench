//! VAD pauses for the unit cutter, from `wb-engine chunks` (decision of 14 Nov 2026, amendment A: pauses come from
//! the engine's own VAD, never from an energy finder; if `chunks` fails the part has no pauses and the job parks
//! with the error, a cut is never guessed).
//!
//! `wb-engine chunks --file PART.wav --out FILE.json --models-dir DIR` runs the same Silero VAD and chunker as
//! `batch` and writes the chunk ranges of one part (times from the start of the part file). A pause is a gap of
//! at least 0.3 s between two chunks, or between the edge of the part and the first or last chunk; chunks that
//! abut or are closer than that are cuts the chunker made inside running speech, kept as fallback cut points.
//!
//! `chunks` is processing like any other engine: [`pauses_for_part`] waits for the [`StartGate`] (the scheduler's
//! pause rules and the 3 GB start reserve) before it starts, and logs the wait and its reason.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use wb_protocol::Event;
use wb_queue::units::Pause;
use wb_supervisor::{Cancel, EngineProcess, Line, Priority, SpawnSpec};

use crate::gate::{GateEvent, StartGate, StartRequest, wait_for_gate};

/// Peak commit assumed for the first `chunks` run on a machine (VAD model, one part decoded to 16 kHz f32).
/// Replaced by the largest peak the engine's job reported once one run has been measured ([`PeakTracker`]); the
/// log says which one a start used.
pub const ASSUMED_CHUNKS_PEAK_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The JSON `wb-engine chunks` writes.
#[derive(Debug, Clone, Deserialize)]
pub struct ChunksDoc {
    pub duration_ms: u64,
    pub chunks: Vec<ChunkRange>,
}

/// What one part gives the cutter, on the session's audio clock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartPauses {
    pub pauses: Vec<Pause>,
    /// Cut points inside running speech (chunk boundaries with less than a pause between them).
    pub fallback: Vec<u64>,
}

pub fn parse_chunks(json: &str) -> Result<ChunksDoc, ChunksError> {
    serde_json::from_str(json).map_err(|e| ChunksError::Bad(format!("chunks output: {e}")))
}

/// Pauses and fallback cut points of one part. `part_start_ms` moves the part's own times to the session clock.
pub fn pauses_of(part_start_ms: u64, doc: &ChunksDoc, min_pause_ms: u64) -> PartPauses {
    let mut chunks = doc.chunks.clone();
    chunks.sort_by_key(|c| (c.start_ms, c.end_ms));
    let mut out = PartPauses::default();
    let edge_gap = |from: u64, to: u64, out: &mut PartPauses| {
        if to > from && to - from >= min_pause_ms {
            out.pauses.push(Pause {
                start_ms: part_start_ms + from,
                end_ms: part_start_ms + to,
            });
        }
    };
    let Some(first) = chunks.first() else {
        edge_gap(0, doc.duration_ms, &mut out);
        return out;
    };
    edge_gap(0, first.start_ms, &mut out);
    let mut end = first.end_ms;
    for c in &chunks[1..] {
        let gap = c.start_ms.saturating_sub(end);
        if gap >= min_pause_ms {
            out.pauses.push(Pause {
                start_ms: part_start_ms + end,
                end_ms: part_start_ms + c.start_ms,
            });
        } else if c.start_ms >= end {
            out.fallback.push(part_start_ms + end + gap / 2);
        }
        end = end.max(c.end_ms);
    }
    edge_gap(end, doc.duration_ms, &mut out);
    out
}

/// Joins pauses that touch or overlap, in order: a silence that runs across the end of one part and the start
/// of the next is one pause, not two.
pub fn merge_pauses(pauses: &mut Vec<Pause>) {
    pauses.sort_by_key(|p| (p.start_ms, p.end_ms));
    let mut merged: Vec<Pause> = Vec::with_capacity(pauses.len());
    for p in pauses.drain(..) {
        match merged.last_mut() {
            Some(last) if p.start_ms <= last.end_ms => last.end_ms = last.end_ms.max(p.end_ms),
            _ => merged.push(p),
        }
    }
    *pauses = merged;
}

#[derive(Debug, thiserror::Error)]
pub enum ChunksError {
    #[error("cancelled")]
    Cancelled,
    #[error("could not start wb-engine chunks: {0}")]
    Start(String),
    #[error("wb-engine chunks failed: {0}")]
    Engine(String),
    #[error("wb-engine chunks took longer than {0} s")]
    Timeout(u64),
    #[error("{0}")]
    Bad(String),
}

/// The largest peak commit a `chunks` run has reported, shared by the workers. Starts at the assumed value.
#[derive(Debug)]
pub struct PeakTracker {
    measured: AtomicU64,
}

impl Default for PeakTracker {
    fn default() -> Self {
        Self {
            measured: AtomicU64::new(0),
        }
    }
}

impl PeakTracker {
    pub fn record(&self, peak_bytes: u64) {
        self.measured.fetch_max(peak_bytes, Ordering::Relaxed);
    }

    /// `(peak used for the start gate, whether it was measured)`.
    pub fn for_gate(&self) -> (u64, bool) {
        let m = self.measured.load(Ordering::Relaxed);
        if m == 0 {
            (ASSUMED_CHUNKS_PEAK_BYTES, false)
        } else {
            (m, true)
        }
    }
}

pub struct ChunksJob<'a> {
    pub exe: &'a Path,
    pub wav: &'a Path,
    pub models_dir: &'a Path,
    /// Where the engine's output and stderr log go.
    pub work_dir: &'a Path,
    pub part: u32,
    pub part_start_ms: u64,
    pub timeout: Duration,
    /// As the app starts its engines: drop `WB_*` and the GPU-selection variables in a release build.
    pub scrub_env: bool,
    pub min_pause_ms: u64,
}

/// What a `chunks` run measured, for the log (every run logs its peak against its limit).
#[derive(Debug, Clone, PartialEq)]
pub struct ChunksStats {
    pub seconds: f64,
    pub peak_memory_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub peak_was_measured: bool,
}

/// Finds the pauses of one closed part. Waits for `gate` first (the reason is reported through `on_gate`), runs
/// `wb-engine chunks` at below-normal priority in a job of its own with the gate's memory limit, and turns its
/// output into pauses on the session clock.
pub fn pauses_for_part(
    job: &ChunksJob<'_>,
    gate: &dyn StartGate,
    peaks: &PeakTracker,
    cancel: &Cancel,
    on_gate: &mut dyn FnMut(GateEvent),
) -> Result<(PartPauses, ChunksStats), ChunksError> {
    let (peak, measured) = peaks.for_gate();
    let request = StartRequest {
        what: format!(
            "chunks for part {} ({} peak)",
            job.part,
            if measured { "measured" } else { "assumed" }
        ),
        peak_commit_bytes: peak,
        gpu_local_bytes: 0,
        other_headroom_bytes: 0,
    };
    let Some(grant) = wait_for_gate(gate, &request, cancel, Duration::from_secs(2), on_gate) else {
        return Err(ChunksError::Cancelled);
    };
    std::fs::create_dir_all(job.work_dir)
        .map_err(|e| ChunksError::Start(format!("{}: {e}", job.work_dir.display())))?;
    let out: PathBuf = job
        .work_dir
        .join(format!("chunks-part-{:04}.json", job.part));
    let _ = std::fs::remove_file(&out);
    let spec = SpawnSpec {
        exe: job.exe.to_path_buf(),
        args: vec![
            "chunks".into(),
            "--file".into(),
            job.wav.to_string_lossy().into_owned(),
            "--out".into(),
            out.to_string_lossy().into_owned(),
            "--models-dir".into(),
            job.models_dir.to_string_lossy().into_owned(),
        ],
        priority: Priority::BelowNormal,
        stderr_log: job.work_dir.join("chunks.log"),
        env: Vec::new(),
        scrub_env: job.scrub_env,
    };
    let started = Instant::now();
    let mut proc = EngineProcess::spawn_limited(&spec, grant.memory_limit_bytes)
        .map_err(|e| ChunksError::Start(e.to_string()))?;
    let mut error: Option<String> = None;
    loop {
        if cancel.is_cancelled() {
            proc.kill();
            return Err(ChunksError::Cancelled);
        }
        if started.elapsed() > job.timeout {
            proc.kill();
            return Err(ChunksError::Timeout(job.timeout.as_secs()));
        }
        match proc.events.recv_timeout(Duration::from_millis(250)) {
            Ok(Line::Event(e)) => {
                if let Event::Error { code, message, .. } = *e {
                    error = Some(format!("{code}: {message}"));
                }
            }
            Ok(Line::Garbled(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let exit = proc.wait();
    let peak_memory_bytes = proc.peak_memory_bytes();
    if let Some(p) = peak_memory_bytes {
        peaks.record(p);
    }
    if exit != Some(0) {
        return Err(ChunksError::Engine(error.unwrap_or_else(|| {
            format!(
                "exit code {}",
                exit.map_or("unknown".to_string(), |c| c.to_string())
            )
        })));
    }
    let text = std::fs::read_to_string(&out)
        .map_err(|e| ChunksError::Bad(format!("chunks wrote no output ({e})")))?;
    let doc = parse_chunks(&text)?;
    let stats = ChunksStats {
        seconds: started.elapsed().as_secs_f64(),
        peak_memory_bytes,
        memory_limit_bytes: grant.memory_limit_bytes,
        peak_was_measured: measured,
    };
    Ok((pauses_of(job.part_start_ms, &doc, job.min_pause_ms), stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(duration_ms: u64, chunks: &[(u64, u64)]) -> ChunksDoc {
        ChunksDoc {
            duration_ms,
            chunks: chunks
                .iter()
                .map(|(s, e)| ChunkRange {
                    start_ms: *s,
                    end_ms: *e,
                })
                .collect(),
        }
    }

    fn p(s: u64, e: u64) -> Pause {
        Pause {
            start_ms: s,
            end_ms: e,
        }
    }

    #[test]
    fn gaps_of_a_third_of_a_second_or_more_are_pauses_on_the_session_clock() {
        // Part 2 starts at 600 s. Chunks 1 s to 5 s and 5.2 s to 9 s (0.2 s apart: not a pause) and 10 s to 14 s.
        let d = doc(15_000, &[(1_000, 5_000), (5_200, 9_000), (10_000, 14_000)]);
        let r = pauses_of(600_000, &d, 300);
        assert_eq!(
            r.pauses,
            vec![
                p(600_000, 601_000),
                p(609_000, 610_000),
                p(614_000, 615_000)
            ]
            .into_iter()
            .filter(|z| z.len_ms() >= 300)
            .collect::<Vec<_>>()
        );
        // The 0.2 s gap is a cut inside running speech: a fallback point at its middle.
        assert_eq!(r.fallback, vec![605_100]);
    }

    #[test]
    fn abutting_chunks_are_fallback_cuts_not_pauses() {
        let d = doc(30_000, &[(0, 15_000), (15_000, 30_000)]);
        let r = pauses_of(0, &d, 300);
        assert!(r.pauses.is_empty());
        assert_eq!(r.fallback, vec![15_000]);
    }

    #[test]
    fn a_part_with_no_speech_is_one_pause() {
        let r = pauses_of(120_000, &doc(60_000, &[]), 300);
        assert_eq!(r.pauses, vec![p(120_000, 180_000)]);
        assert!(r.fallback.is_empty());
    }

    #[test]
    fn silence_across_a_part_boundary_merges_into_one_pause() {
        let a = pauses_of(0, &doc(60_000, &[(1_000, 58_000)]), 300);
        let b = pauses_of(60_000, &doc(60_000, &[(2_000, 59_000)]), 300);
        let mut all = a.pauses;
        all.extend(b.pauses);
        merge_pauses(&mut all);
        assert!(all.contains(&p(58_000, 62_000)), "{all:?}");
    }

    #[test]
    fn merging_joins_overlaps_and_keeps_order() {
        let mut v = vec![p(10, 20), p(0, 5), p(18, 30), p(40, 50)];
        merge_pauses(&mut v);
        assert_eq!(v, vec![p(0, 5), p(10, 30), p(40, 50)]);
    }

    #[test]
    fn unsorted_chunks_and_a_chunk_inside_another_do_not_invent_pauses() {
        let d = doc(20_000, &[(8_000, 12_000), (0, 10_000)]);
        let r = pauses_of(0, &d, 300);
        assert_eq!(r.pauses, vec![p(12_000, 20_000)]);
    }

    #[test]
    fn bad_output_is_an_error_not_a_guess() {
        assert!(matches!(
            parse_chunks("{ not json"),
            Err(ChunksError::Bad(_))
        ));
        assert!(parse_chunks(r#"{"duration_ms":10,"chunks":[]}"#).is_ok());
    }

    #[test]
    fn the_peak_is_assumed_until_one_run_measures_it_then_the_largest_counts() {
        let t = PeakTracker::default();
        assert_eq!(t.for_gate(), (ASSUMED_CHUNKS_PEAK_BYTES, false));
        t.record(300 << 20);
        t.record(200 << 20);
        assert_eq!(t.for_gate(), (300 << 20, true));
    }
}
