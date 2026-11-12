//! The capture margin bookkeeping in the recorder, without a device: each stream's granted device
//! buffer against its own longest gap between callbacks, summed over reconnects, a stall that is
//! still going on when the session stops, and the `capture_stall` events in session.json.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use wb_recorder::capture::{StreamCounters, StreamHandle, record_gap};
use wb_recorder::parts::PartsIndex;
use wb_recorder::recorder::{RecorderConfig, RecorderHandle, RecorderMsg, StopReport};
use wb_recorder::session::{self, InputInfo, SessionMeta, SessionSpec};
use wb_recorder::testutil::TempDir;
use wb_recorder::util::now_ist;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn input() -> InputInfo {
    InputInfo {
        device_name: "test".into(),
        device_id: "test".into(),
        sample_rate: 16_000,
        channels: 1,
        sample_format: "f32".into(),
    }
}

/// Stands in for the driver's data callbacks: counts one every 10 ms, so the recorder sees a stream
/// that is alive. Without it a manual stream looks like a capture thread that never calls back.
struct Ticker {
    on: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Ticker {
    fn start(counters: &Arc<StreamCounters>) -> Self {
        let on = Arc::new(AtomicBool::new(true));
        let (flag, c) = (Arc::clone(&on), Arc::clone(counters));
        let join = std::thread::spawn(move || {
            while flag.load(Ordering::Relaxed) {
                c.callbacks.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        Self {
            on,
            join: Some(join),
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.on.store(false, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Run {
    _root: TempDir,
    dir: std::path::PathBuf,
    rec: RecorderHandle,
    _events: mpsc::Receiver<wb_recorder::events::Event>,
}

fn start(name: &str) -> Result<Run, Box<dyn std::error::Error>> {
    let root = TempDir::new(name)?;
    let spec = SessionSpec::unscheduled(name);
    let dir = session::create_session_dir(root.path(), &spec, now_ist())?;
    let meta = SessionMeta::new(&spec, "test", None, now_ist());
    session::save_meta(&dir, &meta)?;
    let (tx, events) = mpsc::channel();
    let rec = RecorderHandle::spawn(
        RecorderConfig {
            session_dir: dir.clone(),
            part_seconds: 600,
            meta,
            index: PartsIndex::new(),
        },
        tx,
    )?;
    Ok(Run {
        _root: root,
        dir,
        rec,
        _events: events,
    })
}

/// A manual stream that says it asked for `requested` ms and was granted `granted` ms.
fn stream(granted: Option<f64>, gaps_us: &[u64]) -> StreamHandle {
    let (_prod, mut handle) = StreamHandle::manual(input(), 20);
    handle.buffer.requested_ms = Some(1000);
    handle.buffer.granted_ms = granted;
    handle.buffer.granted_frames = granted.map(|g| (g * 16.0) as u32);
    handle.buffer.engine_period_ms = Some(10.0);
    for &us in gaps_us {
        record_gap(&handle.counters, us);
    }
    handle
}

fn finish(run: Run) -> Result<(StopReport, String, String), Box<dyn std::error::Error>> {
    let report = run.rec.stop()?;
    let log = std::fs::read_to_string(run.dir.join("session.log")).unwrap_or_default();
    let json = std::fs::read_to_string(run.dir.join("session.json")).unwrap_or_default();
    Ok((report, log, json))
}

#[test]
fn each_stream_is_measured_against_its_own_grant_and_the_session_keeps_the_smallest_margin()
-> TestResult {
    let run = start("margin-two-streams")?;
    // A 1,000 ms grant with a 600 ms stall (margin 400), then, after a reconnect, a 200 ms grant
    // with a 150 ms stall (margin 50). A single grant against the single longest gap would say
    // 200 - 600 = -400 or 1000 - 150 = 850; neither is true.
    let a = stream(Some(1000.0), &[10_000, 600_000, 10_000]);
    let _tick_a = Ticker::start(&a.counters);
    run.rec.send(RecorderMsg::Stream(a))?;
    std::thread::sleep(Duration::from_millis(700));
    let b = stream(Some(200.0), &[10_000, 150_000, 10_000]);
    let _tick_b = Ticker::start(&b.counters);
    run.rec.send(RecorderMsg::Stream(b))?;
    std::thread::sleep(Duration::from_millis(700));
    let (report, log, json) = finish(run)?;
    assert_eq!(
        report.capture_buffers.len(),
        2,
        "{:?}",
        report.capture_buffers
    );
    assert!(report.capture_buffers[0].contains("granted 1000.0 ms"));
    assert!(report.capture_buffers[1].contains("granted 200.0 ms"));
    assert_eq!(report.capture_buffer_granted_ms, Some(200.0));
    assert!((report.capture_max_callback_gap_ms - 600.0).abs() < 1e-6);
    let margin = report.capture_margin_ms.ok_or("no margin")?;
    assert!((margin - 50.0).abs() < 1.0, "margin {margin}");
    assert_eq!(report.capture_gaps_ge_100ms, 2);
    assert_eq!(report.capture_gaps_ge_500ms, 1);
    assert_eq!(report.capture_gaps_ge_50ms, 2);
    // The stalls are events in session.json and lines in session.log, not only in the final report.
    assert!(
        json.contains("capture_stall"),
        "session.json has no capture_stall event"
    );
    assert!(log.contains("capture_stall"));
    assert!(log.contains("capture_buffer"));
    assert!(log.contains("granted_buffer_ms=1000"));
    assert!(log.contains("granted_buffer_ms=200"));
    Ok(())
}

#[test]
fn a_stream_whose_grant_could_not_be_read_makes_the_margin_unknown() -> TestResult {
    let run = start("margin-unknown")?;
    let a = stream(Some(1000.0), &[10_000, 100_000]);
    let _tick_a = Ticker::start(&a.counters);
    run.rec.send(RecorderMsg::Stream(a))?;
    std::thread::sleep(Duration::from_millis(400));
    let b = stream(None, &[10_000, 20_000]);
    let _tick_b = Ticker::start(&b.counters);
    run.rec.send(RecorderMsg::Stream(b))?;
    std::thread::sleep(Duration::from_millis(400));
    let (report, _log, _json) = finish(run)?;
    assert_eq!(
        report.capture_margin_ms, None,
        "never a margin against another stream's grant"
    );
    assert_eq!(report.capture_buffer_granted_ms, Some(1000.0));
    Ok(())
}

#[test]
fn a_stall_still_going_on_at_stop_counts() -> TestResult {
    let run = start("margin-in-progress")?;
    // No callback ever arrives (the manual stream's counter stays at 0): the capture thread is
    // "away" from the moment the stream opens. After about a second the session stops.
    run.rec
        .send(RecorderMsg::Stream(stream(Some(1500.0), &[])))?;
    std::thread::sleep(Duration::from_millis(1_000));
    let (report, log, json) = finish(run)?;
    assert!(
        report.capture_max_callback_gap_ms >= 900.0,
        "the stall in progress is the longest gap: {}",
        report.capture_max_callback_gap_ms
    );
    let margin = report.capture_margin_ms.ok_or("no margin")?;
    assert!(margin < 600.0, "margin {margin}");
    assert!(log.contains("still_going_on=true"), "{log}");
    assert!(json.contains("capture_stall"));
    Ok(())
}
