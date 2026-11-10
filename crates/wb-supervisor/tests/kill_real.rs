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

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use wb_protocol::{BatchRequest, Device, Event, ModelRef, SegmentEvent, Settings, Source};
use wb_supervisor::{Committer, Notice, Observer, Policy, run_batch};

fn env(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set")))
}

fn request(wav: &PathBuf, model: &PathBuf, vad: &PathBuf, threads: u32) -> BatchRequest {
    let m = |id: &str, p: &PathBuf| ModelRef {
        id: id.into(),
        path: p.to_string_lossy().into_owned(),
    };
    let settings = Settings {
        previous_text_context: false,
        sentence_split: false,
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
}

impl Observer for Killer {
    fn event(&mut self, e: &Event) {
        if let Event::Segment(s) = e {
            self.seen += 1;
            self.last_end_ms = s.end_ms;
            if self.at.contains(&self.seen) {
                let status = Command::new("taskkill")
                    .args(["/F", "/IM", &self.exe_name])
                    .output()
                    .expect("taskkill");
                assert!(status.status.success(), "taskkill failed: {status:?}");
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
        ..Policy::default()
    };

    // 1. Uninterrupted baseline.
    let base_path = out.join("baseline.jsonl");
    let _ = std::fs::remove_file(&base_path);
    let mut base = Committer::open(&base_path).unwrap();
    let t0 = Instant::now();
    let mut quiet = Killer {
        exe_name: exe_name.clone(),
        at: vec![],
        seen: 0,
        killed: vec![],
        notices: vec![],
        last_end_ms: 0,
    };
    let rep = run_batch(
        &exe,
        request(&wav, &model, &vad, threads),
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
    let mut killer = Killer {
        exe_name,
        at: at.clone(),
        seen: 0,
        killed: vec![],
        notices: vec![],
        last_end_ms: 0,
    };
    let t1 = Instant::now();
    let rep = run_batch(
        &exe,
        request(&wav, &model, &vad, threads),
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
