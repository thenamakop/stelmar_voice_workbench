//! Resume after kills at random points (Phase 4 Verification: property test). Units finish in any order, the
//! app commits them in audio order, and the process dies at a random moment: between two commits, in the middle
//! of a commit's write (a torn last line), or after the JSONL write but before SQLite has the rows. After a
//! restart the unfinished units run again; nothing is lost, nothing is repeated, the order is the audio order.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use wb_queue::order::OrderBuffer;
use wb_queue::revision::{Transcript, jsonl_path};
use wb_queue::store::{Device, Segment};

const UNIT_MS: u64 = 300_000;
const SEGS: u64 = 5;

fn seg(start: u64, end: u64) -> Segment {
    Segment {
        revision: "post-1".into(),
        start_ms: start,
        end_ms: end,
        text: format!("segment at {start}"),
        lang: "hi".into(),
        lang_prob: 0.9,
        mixed: false,
        dir: "ltr".into(),
        avg_logprob: -0.2,
        no_speech_prob: 0.0,
        model: "m".into(),
        device: Device::Cpu,
        flags: vec![],
        hint_text: None,
        created_at: String::new(),
    }
}

/// The segments of unit `idx`: five of them, the last ending 100 ms before the unit does (silence at the end).
fn unit_segments(idx: u32) -> Vec<Segment> {
    let base = u64::from(idx) * UNIT_MS;
    let step = UNIT_MS / SEGS;
    (0..SEGS)
        .map(|i| {
            seg(
                base + i * step,
                base + (i + 1) * step - if i + 1 == SEGS { 100 } else { 0 },
            )
        })
        .collect()
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wb-queue-resume-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[derive(Debug, Clone, Copy)]
enum Kill {
    /// Between two commits.
    Clean,
    /// Part of the next unit's first lines reached the file, the last of them torn.
    TornWrite,
    /// The whole next unit reached the file, but not SQLite.
    BeforeSqlite,
}

fn append_raw(dir: &Path, bytes: &[u8]) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(jsonl_path(dir, "post-1"))
        .unwrap();
    f.write_all(bytes).unwrap();
}

/// Runs the pipeline for `units` units. Finishing order is `order`; the process is killed after `kill_after`
/// commits (if given) the way `kill` says, then restarted and run to the end.
fn run(dir: &Path, units: u32, order: &[u32], kill: Option<(usize, Kill)>) {
    let mut t = Transcript::open(dir).unwrap();
    let mut buf: OrderBuffer<Vec<Segment>> = OrderBuffer::new(0, 0);
    let mut commits = 0usize;
    let mut killed = false;
    'outer: for &idx in order {
        buf.finished(idx, (u64::from(idx) + 1) * UNIT_MS, unit_segments(idx))
            .unwrap();
        while let Some((i, _end, segs)) = buf.pop_ready() {
            if let Some((after, how)) = kill
                && !killed
                && commits == after
            {
                // The process dies here.
                match how {
                    Kill::Clean => {}
                    Kill::TornWrite => {
                        let mut bytes = Vec::new();
                        for s in segs.iter().take(2) {
                            serde_json::to_writer(&mut bytes, s).unwrap();
                            bytes.push(b'\n');
                        }
                        let mut third = serde_json::to_vec(&segs[2]).unwrap();
                        third.truncate(third.len() / 2);
                        bytes.extend(third);
                        append_raw(dir, &bytes);
                    }
                    Kill::BeforeSqlite => {
                        let mut bytes = Vec::new();
                        for s in &segs {
                            serde_json::to_writer(&mut bytes, s).unwrap();
                            bytes.push(b'\n');
                        }
                        append_raw(dir, &bytes);
                    }
                }
                killed = true;
                drop(t);
                break 'outer;
            }
            t.commit("post-1", &segs).unwrap();
            commits += 1;
            let _ = i;
        }
    }
    if killed {
        // Restart: reopen (recovery), then run every unit whose segments are not all committed, in order.
        let mut t = Transcript::open(dir).unwrap();
        let committed = t.committed_end_ms("post-1");
        for idx in 0..units {
            let segs = unit_segments(idx);
            if segs.last().is_some_and(|s| s.end_ms > committed) {
                t.commit("post-1", &segs).unwrap();
            }
        }
    }
}

fn expected(units: u32) -> Vec<Segment> {
    (0..units).flat_map(unit_segments).collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn a_kill_at_any_point_loses_nothing_and_repeats_nothing(
        order in Just((0u32..12).collect::<Vec<_>>()).prop_shuffle(),
        kill_after in 0usize..12,
        how in prop_oneof![Just(Kill::Clean), Just(Kill::TornWrite), Just(Kill::BeforeSqlite)],
        case in 0u32..1_000_000,
    ) {
        let dir = temp(&format!("p{case}"));
        run(&dir, 12, &order, Some((kill_after, how)));
        let t = Transcript::open(&dir).unwrap();
        let want = expected(12);
        // SQLite has exactly the segments of an uninterrupted run, in audio order.
        let got = t.store().segments("post-1", 0, u64::MAX).unwrap();
        prop_assert_eq!(&got, &want);
        // So does the JSONL, line for line.
        let text = std::fs::read_to_string(jsonl_path(&dir, "post-1")).unwrap();
        let lines: Vec<Segment> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        prop_assert_eq!(&lines, &want);
        prop_assert!(text.ends_with('\n'));
        // Audio order, no overlap.
        for w in lines.windows(2) {
            prop_assert!(w[1].start_ms >= w[0].end_ms);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn an_uninterrupted_run_equals_the_expectation() {
    let dir = temp("clean");
    let order: Vec<u32> = (0..12).rev().collect();
    run(&dir, 12, &order, None);
    let t = Transcript::open(&dir).unwrap();
    assert_eq!(
        t.store().segments("post-1", 0, u64::MAX).unwrap(),
        expected(12)
    );
}
