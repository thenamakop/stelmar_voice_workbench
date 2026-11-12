//! Phase 2 kill test with the REAL engine: the supervisor restarts `wb-engine batch` after it is killed at
//! five random points and the committed transcript equals an uninterrupted run (no segment lost, none
//! repeated, same text). Ignored by default: it needs a built engine, a model and a 5-minute WAV, so
//! `scripts/gates/kill-engine.ps1` sets the environment and runs it under an emulation profile.
//!
//! Environment:
//!   WB_KILLTEST_ENGINE  the engine exe, copied to a name no other engine has (it is killed by name)
//!   WB_KILLTEST_WAV     16 kHz mono WAV of about five minutes
//!   WB_KILLTEST_MODEL   ggml model file (small)      WB_KILLTEST_VAD  Silero VAD ggml file
//!   WB_KILLTEST_OUT     folder for the two transcripts and the report
//!   WB_KILLTEST_THREADS engine threads (default 3)   WB_KILLTEST_SEED  seed (default 20261110)
//!
//! Previous-text context is off so that a restart cannot change the text of the chunk that follows it: the
//! committed text must then be identical to the uninterrupted run, word for word.
//!
//! Four tests share this setup:
//!   * `five_random_kills_leave_the_transcript_unchanged`: five kills between chunks.
//!   * `kills_inside_the_second_half_of_a_split_chunk_lose_and_repeat_no_half`: the engine is told to split
//!     every chunk of four seconds or more (`debug_force_split`), and is killed right after it reports the
//!     first half done, i.e. while it decodes the second half. Nothing of that chunk is committed yet, so the
//!     restart redoes the whole chunk: no half is lost, none appears twice.
//!   * `a_decode_of_over_two_minutes_is_not_a_hang`: large-v3 q5_0 on a Hindi clip with 30 s chunks under the
//!     default 60 s watchdog; the engine sends heartbeats from whisper's callbacks, so no restart happens.
//!     Environment: WB_SLOWTEST_ENGINE, WB_SLOWTEST_WAV, WB_SLOWTEST_MODEL (large-v3 q5_0), WB_SLOWTEST_VAD,
//!     WB_SLOWTEST_OUT, WB_SLOWTEST_THREADS (default 3).
//!   * `a_language_detection_pass_longer_than_the_watchdog_is_not_a_hang`: languages en+hi with detection on and
//!     a 20 s watchdog; the detection pass has no whisper callback, so the engine holds the heartbeats for it.
//!     Same environment as the slow case plus WB_DETECT_WAV (a short Hindi clip) and WB_DETECT_OUT.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use wb_protocol::{BatchRequest, Device, Event, ModelRef, SegmentEvent, Settings, Source};
use wb_supervisor::{Committer, Notice, Observer, Policy, run_batch};

fn env(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set")))
}

fn request(wav: &Path, model: &Path, vad: &Path, threads: u32, force_split: bool) -> BatchRequest {
    let m = |id: &str, p: &Path| ModelRef {
        id: id.into(),
        path: p.to_string_lossy().into_owned(),
    };
    let settings = Settings {
        previous_text_context: false,
        sentence_split: false,
        debug_force_split: force_split,
        ..Settings::default()
    };
    BatchRequest {
        source: Source::File {
            path: wav.to_string_lossy().into_owned(),
        },
        units: vec![],
        models_by_lang: BTreeMap::from([("en".into(), m("small", model))]),
        vad_model: m("silero-v6.2.0", vad),
        languages: vec!["en".into()],
        lang_log: vec![],
        settings,
        device: Device::Cpu,
        threads,
        resume_from_ms: 0,
        detect_language: false,
    }
}

/// Kills the engine (by its unique file name) after chosen segment counts, i.e. while it is decoding
/// the next chunk. The supervisor has committed the segment before the observer sees it.
struct Killer {
    exe_name: String,
    at: Vec<u64>,
    seen: u64,
    killed: Vec<(u64, u64)>,
    notices: Vec<Notice>,
    last_end_ms: u64,
    /// Count and kill on `chunk_half_done` warnings (inside the second half) instead of on segments.
    on_half: bool,
    halves: u64,
}

impl Killer {
    fn new(exe_name: &str, at: Vec<u64>, on_half: bool) -> Self {
        Self {
            exe_name: exe_name.to_string(),
            at,
            seen: 0,
            killed: vec![],
            notices: vec![],
            last_end_ms: 0,
            on_half,
            halves: 0,
        }
    }
    fn kill(&self) {
        let status = Command::new("taskkill")
            .args(["/F", "/IM", &self.exe_name])
            .output()
            .expect("taskkill");
        assert!(status.status.success(), "taskkill failed: {status:?}");
    }
}

impl Observer for Killer {
    fn event(&mut self, e: &Event) {
        if let Event::Warning { code, .. } = e
            && self.on_half
            && code == "chunk_half_done"
        {
            self.halves += 1;
            if self.at.contains(&self.halves) {
                self.kill();
                self.killed.push((self.halves, self.last_end_ms));
            }
        }
        if let Event::Segment(s) = e {
            self.seen += 1;
            self.last_end_ms = s.end_ms;
            if !self.on_half && self.at.contains(&self.seen) {
                self.kill();
                self.killed.push((self.seen, s.end_ms));
            }
        }
    }
    fn notice(&mut self, n: &Notice) {
        self.notices.push(n.clone());
    }
}

fn read_segments(path: &std::path::Path) -> Vec<SegmentEvent> {
    std::fs::read_to_string(path)
        .expect("transcript")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("segment line"))
        .collect()
}

/// Small deterministic generator (no extra dependency).
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

#[test]
#[ignore = "needs a built engine, a model and a 5-minute WAV; run scripts/gates/kill-engine.ps1"]
fn five_random_kills_leave_the_transcript_unchanged() {
    let exe = env("WB_KILLTEST_ENGINE");
    let wav = env("WB_KILLTEST_WAV");
    let model = env("WB_KILLTEST_MODEL");
    let vad = env("WB_KILLTEST_VAD");
    let out = env("WB_KILLTEST_OUT");
    let threads: u32 = std::env::var("WB_KILLTEST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let mut seed: u64 = std::env::var("WB_KILLTEST_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_261_110);
    let exe_name = exe.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::create_dir_all(&out).unwrap();
    let policy = Policy {
        watchdog: Duration::from_secs(120),
        load_timeout: Duration::from_secs(120),
        max_restarts: 12,
        backoff_s: vec![0],
        scrub_env: false,
        ..Policy::default()
    };

    // 1. Uninterrupted baseline.
    let base_path = out.join("baseline.jsonl");
    let _ = std::fs::remove_file(&base_path);
    let mut base = Committer::open(&base_path).unwrap();
    let t0 = Instant::now();
    let mut quiet = Killer::new(&exe_name, vec![], false);
    let rep = run_batch(
        &exe,
        request(&wav, &model, &vad, threads, false),
        &mut base,
        &out.join("work-baseline"),
        &policy,
        &mut quiet,
    )
    .expect("baseline run");
    let baseline_s = t0.elapsed().as_secs_f64();
    assert_eq!(rep.restarts, 0, "the baseline must not need a restart: {:?}", quiet.notices);
    let baseline = read_segments(&base_path);
    assert!(baseline.len() >= 12, "too few segments for five kills: {}", baseline.len());

    // 2. Five random kill points among segments 2 .. n-2, distinct.
    let n = baseline.len() as u64;
    let mut at: Vec<u64> = Vec::new();
    while at.len() < 5 {
        let k = 2 + lcg(&mut seed) % (n - 4);
        if !at.contains(&k) {
            at.push(k);
        }
    }
    at.sort_unstable();

    // 3. The same job, killed at those points.
    let kill_path = out.join("killed.jsonl");
    let _ = std::fs::remove_file(&kill_path);
    let mut committer = Committer::open(&kill_path).unwrap();
    let mut killer = Killer::new(&exe_name, at.clone(), false);
    let t1 = Instant::now();
    let rep = run_batch(
        &exe,
        request(&wav, &model, &vad, threads, false),
        &mut committer,
        &out.join("work-killed"),
        &policy,
        &mut killer,
    )
    .expect("killed run");
    let killed_s = t1.elapsed().as_secs_f64();
    let after = read_segments(&kill_path);

    // 4. Compare.
    let mut report = String::new();
    writeln!(report, "segments in the uninterrupted run: {}", baseline.len()).unwrap();
    writeln!(report, "kill points (after segment number, audio ms): {:?}", killer.killed).unwrap();
    writeln!(report, "restarts: {} (kills sent: {})", rep.restarts, killer.killed.len()).unwrap();
    writeln!(report, "segments after the kills: {}; repeats refused by the committer: {}", after.len(), committer.repeats_refused()).unwrap();
    writeln!(report, "wall time: uninterrupted {baseline_s:.0} s, with kills {killed_s:.0} s").unwrap();
    let mut problems: Vec<String> = Vec::new();
    if killer.killed.len() != 5 {
        problems.push(format!("sent {} kills, wanted 5", killer.killed.len()));
    }
    if rep.restarts < 5 {
        problems.push(format!("only {} restarts after 5 kills", rep.restarts));
    }
    let times = |v: &[SegmentEvent]| v.iter().map(|s| (s.start_ms, s.end_ms)).collect::<Vec<_>>();
    if times(&baseline) != times(&after) {
        problems.push(format!(
            "segment times differ: {} against {}",
            baseline.len(),
            after.len()
        ));
    }
    let text_diffs = baseline
        .iter()
        .zip(after.iter())
        .filter(|(a, b)| a.text != b.text)
        .count();
    if text_diffs > 0 {
        problems.push(format!("{text_diffs} segments have different text"));
    }
    if after.windows(2).any(|w| w[1].start_ms < w[0].end_ms) {
        problems.push("segments overlap".into());
    }
    writeln!(report, "segment times equal to the uninterrupted run: {}", times(&baseline) == times(&after)).unwrap();
    writeln!(report, "segments with different text: {text_diffs}").unwrap();
    writeln!(report, "notices: {:?}", killer.notices).unwrap();
    writeln!(report, "result: {}", if problems.is_empty() { "PASS".to_string() } else { format!("FAIL: {}", problems.join("; ")) }).unwrap();
    std::fs::write(out.join("report.txt"), &report).unwrap();
    println!("{report}");
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
#[ignore = "needs a built engine, a model and a 5-minute WAV; run scripts/gates/kill-engine.ps1"]
fn kills_inside_the_second_half_of_a_split_chunk_lose_and_repeat_no_half() {
    let exe = env("WB_KILLTEST_ENGINE");
    let wav = env("WB_KILLTEST_WAV");
    let model = env("WB_KILLTEST_MODEL");
    let vad = env("WB_KILLTEST_VAD");
    let out = env("WB_KILLTEST_OUT").join("split");
    let threads: u32 = std::env::var("WB_KILLTEST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let mut seed: u64 = std::env::var("WB_KILLTEST_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_261_110);
    let exe_name = exe.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::create_dir_all(&out).unwrap();
    let policy = Policy {
        watchdog: Duration::from_secs(120),
        load_timeout: Duration::from_secs(120),
        max_restarts: 12,
        backoff_s: vec![0],
        scrub_env: false,
        // The engine honours `debug_force_split` only with this variable (batch.rs `test_hooks_enabled`).
        engine_env: vec![("WB_ENGINE_TEST_HOOKS".into(), "1".into())],
        ..Policy::default()
    };

    // 1. Uninterrupted baseline with every chunk of 4 s or more forced through the split path.
    let base_path = out.join("baseline.jsonl");
    let _ = std::fs::remove_file(&base_path);
    let mut base = Committer::open(&base_path).unwrap();
    let mut quiet = Killer::new(&exe_name, vec![], true);
    let t0 = Instant::now();
    let rep = run_batch(
        &exe,
        request(&wav, &model, &vad, threads, true),
        &mut base,
        &out.join("work-baseline"),
        &policy,
        &mut quiet,
    )
    .expect("baseline run");
    let baseline_s = t0.elapsed().as_secs_f64();
    assert_eq!(rep.restarts, 0, "the baseline must not need a restart: {:?}", quiet.notices);
    let baseline = read_segments(&base_path);
    let split_chunks = quiet.halves;
    assert!(split_chunks >= 8, "too few split chunks for three kills: {split_chunks}");

    // 2. Three kill points among the split chunks (the n-th 'first half done' report), 2 .. n-2, never
    //    adjacent: the redone chunk reports its first half again and must not be hit a second time.
    let mut at: Vec<u64> = Vec::new();
    let mut guard = 0;
    while at.len() < 3 && guard < 1000 {
        guard += 1;
        let k = 2 + lcg(&mut seed) % (split_chunks - 4);
        if at.iter().all(|&x| x.abs_diff(k) >= 3) {
            at.push(k);
        }
    }
    at.sort_unstable();
    assert_eq!(at.len(), 3, "could not draw three kill points");

    // 3. The same job, killed inside the second half at those points.
    let kill_path = out.join("killed.jsonl");
    let _ = std::fs::remove_file(&kill_path);
    let mut committer = Committer::open(&kill_path).unwrap();
    let mut killer = Killer::new(&exe_name, at.clone(), true);
    let t1 = Instant::now();
    let rep = run_batch(
        &exe,
        request(&wav, &model, &vad, threads, true),
        &mut committer,
        &out.join("work-killed"),
        &policy,
        &mut killer,
    )
    .expect("killed run");
    let killed_s = t1.elapsed().as_secs_f64();
    let after = read_segments(&kill_path);

    // 4. Compare.
    let times = |v: &[SegmentEvent]| v.iter().map(|s| (s.start_ms, s.end_ms)).collect::<Vec<_>>();
    let text_diffs = baseline
        .iter()
        .zip(after.iter())
        .filter(|(a, b)| a.text != b.text)
        .count();
    let mut problems: Vec<String> = Vec::new();
    if killer.killed.len() != 3 {
        problems.push(format!("sent {} kills, wanted 3", killer.killed.len()));
    }
    if rep.restarts < 3 {
        problems.push(format!("only {} restarts after 3 kills", rep.restarts));
    }
    if times(&baseline) != times(&after) {
        problems.push(format!("segment times differ: {} against {}", baseline.len(), after.len()));
    }
    if text_diffs > 0 {
        problems.push(format!("{text_diffs} segments have different text"));
    }
    if after.windows(2).any(|w| w[1].start_ms < w[0].end_ms) {
        problems.push("segments overlap".into());
    }
    if committer.repeats_refused() != 0 {
        problems.push(format!("the committer refused {} repeated segments", committer.repeats_refused()));
    }
    // Nothing of the killed chunk was committed: at each death the cursor stood where the previous chunk
    // ended, i.e. at the end of the last segment the observer had been shown.
    let cursors: Vec<u64> = killer
        .notices
        .iter()
        .filter_map(|n| match n {
            Notice::Died { committed_until_ms, .. } => Some(*committed_until_ms),
            _ => None,
        })
        .collect();
    let last_ends: Vec<u64> = killer.killed.iter().map(|k| k.1).collect();
    if cursors != last_ends {
        problems.push(format!("cursor at each death {cursors:?} differs from the last committed segment end {last_ends:?}"));
    }
    let mut report = String::new();
    writeln!(report, "split case: every chunk of 4 s or more is cut at its deepest pause and decoded as two halves (debug_force_split)").unwrap();
    writeln!(report, "chunks that reported a first half done in the uninterrupted run: {split_chunks}; segments: {}", baseline.len()).unwrap();
    writeln!(report, "kill points (number of the first-half-done report, committed audio ms at that time): {:?}", killer.killed).unwrap();
    writeln!(report, "the cursor at each death equalled the end of the last committed segment ({last_ends:?}): nothing of the killed chunk was committed").unwrap();
    writeln!(report, "restarts: {} (kills sent: {})", rep.restarts, killer.killed.len()).unwrap();
    writeln!(report, "segments after the kills: {}; repeats refused by the committer: {}", after.len(), committer.repeats_refused()).unwrap();
    writeln!(report, "wall time: uninterrupted {baseline_s:.0} s, with kills {killed_s:.0} s").unwrap();
    writeln!(report, "segment times equal to the uninterrupted run: {}", times(&baseline) == times(&after)).unwrap();
    writeln!(report, "segments with different text: {text_diffs}").unwrap();
    writeln!(report, "notices: {:?}", killer.notices).unwrap();
    writeln!(report, "result: {}", if problems.is_empty() { "PASS".to_string() } else { format!("FAIL: {}", problems.join("; ")) }).unwrap();
    std::fs::write(out.join("report.txt"), &report).unwrap();
    println!("{report}");
    assert!(problems.is_empty(), "{problems:?}");
}

/// Records the longest silence between committed chunks and every notice.
struct Gaps {
    started: Instant,
    first_segment: Option<Duration>,
    last: Instant,
    longest: Duration,
    segments: u64,
    heartbeats: u64,
    notices: Vec<Notice>,
}

impl Observer for Gaps {
    fn event(&mut self, e: &Event) {
        match e {
            Event::Segment(_) => {
                self.segments += 1;
                if self.first_segment.is_none() {
                    self.first_segment = Some(self.started.elapsed());
                }
                self.longest = self.longest.max(self.last.elapsed());
                self.last = Instant::now();
            }
            Event::Heartbeat { .. } => self.heartbeats += 1,
            _ => {}
        }
    }
    fn notice(&mut self, n: &Notice) {
        self.notices.push(n.clone());
    }
}

#[test]
#[ignore = "needs a built engine, large-v3 q5_0 and a Hindi clip; run scripts/gates/kill-engine.ps1"]
fn a_decode_of_over_two_minutes_is_not_a_hang() {
    let exe = env("WB_SLOWTEST_ENGINE");
    let wav = env("WB_SLOWTEST_WAV");
    let model = env("WB_SLOWTEST_MODEL");
    let vad = env("WB_SLOWTEST_VAD");
    let out = env("WB_SLOWTEST_OUT");
    let threads: u32 = std::env::var("WB_SLOWTEST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    std::fs::create_dir_all(&out).unwrap();
    // The default policy: 60 s watchdog, so a restart would show on any silence over a minute.
    let policy = Policy {
        load_timeout: Duration::from_secs(300),
        max_restarts: 2,
        backoff_s: vec![0],
        scrub_env: false,
        ..Policy::default()
    };
    assert_eq!(policy.watchdog, Duration::from_secs(60), "this test runs the default watchdog");
    let mut req = request(&wav, &model, &vad, threads, false);
    req.models_by_lang = BTreeMap::from([(
        "hi".to_string(),
        ModelRef {
            id: "large-v3-q5_0".into(),
            path: model.to_string_lossy().into_owned(),
        },
    )]);
    req.languages = vec!["hi".into()];
    req.settings.max_chunk_s = 30.0;
    let path = out.join("slow.jsonl");
    let _ = std::fs::remove_file(&path);
    let mut committer = Committer::open(&path).unwrap();
    let mut gaps = Gaps {
        started: Instant::now(),
        first_segment: None,
        last: Instant::now(),
        longest: Duration::ZERO,
        segments: 0,
        heartbeats: 0,
        notices: vec![],
    };
    let t0 = Instant::now();
    let rep = run_batch(&exe, req, &mut committer, &out.join("work"), &policy, &mut gaps).expect("slow run");
    let wall = t0.elapsed().as_secs_f64();
    let hung = gaps.notices.iter().any(|n| matches!(n, Notice::Hung { .. }));
    let mut problems: Vec<String> = Vec::new();
    if rep.restarts != 0 || hung {
        problems.push(format!("the engine was restarted ({} restarts, hung: {hung})", rep.restarts));
    }
    if gaps.longest < Duration::from_secs(120) {
        problems.push(format!(
            "the longest decode took {:.0} s, under the 120 s this test must exceed",
            gaps.longest.as_secs_f64()
        ));
    }
    if gaps.heartbeats == 0 {
        problems.push("no heartbeat was seen".into());
    }
    let mut report = String::new();
    writeln!(report, "slow-decode case: large-v3 q5_0, Hindi, {threads} threads, 30 s chunks, default watchdog {} s", policy.watchdog.as_secs()).unwrap();
    writeln!(report, "longest time between two committed chunks (all decodes of one chunk, retries included; the first is counted from the start of the run): {:.0} s", gaps.longest.as_secs_f64()).unwrap();
    writeln!(report, "heartbeats seen by the supervisor: {}; segments committed: {}", gaps.heartbeats, gaps.segments).unwrap();
    writeln!(report, "restarts: {}; watchdog kills: {}", rep.restarts, u32::from(hung)).unwrap();
    writeln!(report, "wall time: {wall:.0} s").unwrap();
    writeln!(report, "notices: {:?}", gaps.notices).unwrap();
    writeln!(report, "result: {}", if problems.is_empty() { "PASS".to_string() } else { format!("FAIL: {}", problems.join("; ")) }).unwrap();
    std::fs::write(out.join("report.txt"), &report).unwrap();
    println!("{report}");
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
#[ignore = "needs a built engine, large-v3 q5_0 and a short Hindi clip; run scripts/gates/kill-engine.ps1"]
fn a_language_detection_pass_longer_than_the_watchdog_is_not_a_hang() {
    let exe = env("WB_SLOWTEST_ENGINE");
    let wav = env("WB_DETECT_WAV");
    let model = env("WB_SLOWTEST_MODEL");
    let vad = env("WB_SLOWTEST_VAD");
    let out = env("WB_DETECT_OUT");
    let threads: u32 = std::env::var("WB_SLOWTEST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    std::fs::create_dir_all(&out).unwrap();
    // A watchdog of 20 s: the detection pass has no whisper callback, so without the engine's hold the
    // heartbeats would stop 20 s after the pass starts and the engine would be killed after 40 s of silence.
    let policy = Policy {
        watchdog: Duration::from_secs(20),
        load_timeout: Duration::from_secs(300),
        max_restarts: 2,
        backoff_s: vec![0],
        scrub_env: false,
        ..Policy::default()
    };
    let m = |lang: &str| {
        (
            lang.to_string(),
            ModelRef {
                id: "large-v3-q5_0".into(),
                path: model.to_string_lossy().into_owned(),
            },
        )
    };
    let mut req = request(&wav, &model, &vad, threads, false);
    req.models_by_lang = BTreeMap::from([m("en"), m("hi")]);
    req.languages = vec!["en".into(), "hi".into()];
    req.detect_language = true;
    let path = out.join("detect.jsonl");
    let _ = std::fs::remove_file(&path);
    let mut committer = Committer::open(&path).unwrap();
    let mut gaps = Gaps {
        started: Instant::now(),
        first_segment: None,
        last: Instant::now(),
        longest: Duration::ZERO,
        segments: 0,
        heartbeats: 0,
        notices: vec![],
    };
    let t0 = Instant::now();
    let rep = match run_batch(&exe, req, &mut committer, &out.join("work"), &policy, &mut gaps) {
        Ok(rep) => rep,
        Err(e) => {
            let msg = format!("the run failed: {e:?}; notices: {:?}; heartbeats: {}", gaps.notices, gaps.heartbeats);
            std::fs::write(out.join("report.txt"), format!("result: FAIL: {msg}
")).unwrap();
            panic!("{msg}");
        }
    };
    let wall = t0.elapsed().as_secs_f64();
    let first = gaps.first_segment.unwrap_or_default();
    let hung = gaps.notices.iter().any(|n| matches!(n, Notice::Hung { .. }));
    let mut problems: Vec<String> = Vec::new();
    if rep.restarts != 0 || hung {
        problems.push(format!("the engine was restarted ({} restarts, hung: {hung})", rep.restarts));
    }
    if first < Duration::from_secs(40) {
        problems.push(format!(
            "the first chunk was committed after {:.0} s, under the 40 s this test must exceed to prove anything",
            first.as_secs_f64()
        ));
    }
    if gaps.segments == 0 {
        problems.push("no segment was committed".into());
    }
    let mut report = String::new();
    writeln!(report, "detection-on case: large-v3 q5_0 for en and hi, languages en+hi with detection on, {threads} threads, watchdog {} s (the default is 60 s)", policy.watchdog.as_secs()).unwrap();
    writeln!(report, "time to the first committed chunk (load, language detection, decode): {:.0} s; it must exceed 40 s (20 s watchdog + 20 s heartbeat freshness) for the test to prove the hold", first.as_secs_f64()).unwrap();
    writeln!(report, "heartbeats seen by the supervisor: {}; segments committed: {}", gaps.heartbeats, gaps.segments).unwrap();
    writeln!(report, "restarts: {}; watchdog kills: {}", rep.restarts, u32::from(hung)).unwrap();
    writeln!(report, "wall time: {wall:.0} s").unwrap();
    writeln!(report, "notices: {:?}", gaps.notices).unwrap();
    writeln!(report, "result: {}", if problems.is_empty() { "PASS".to_string() } else { format!("FAIL: {}", problems.join("; ")) }).unwrap();
    std::fs::write(out.join("report.txt"), &report).unwrap();
    println!("{report}");
    assert!(problems.is_empty(), "{problems:?}");
}
