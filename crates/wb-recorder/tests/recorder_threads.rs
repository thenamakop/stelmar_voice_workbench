//! The recorder thread end to end, fed by a manual stream instead of a device: exact samples
//! across part boundaries, pause and resume, input loss, and isolation from panics elsewhere.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use ringbuf::traits::Producer;
use wb_recorder::capture::StreamHandle;
use wb_recorder::events::Event;
use wb_recorder::parts::{self, PartsIndex};
use wb_recorder::recorder::{PanicPoint, RecorderConfig, RecorderHandle, RecorderMsg};
use wb_recorder::session::{self, InputInfo, SessionMeta, SessionSpec};
use wb_recorder::testutil::TempDir;
use wb_recorder::util::now_ist;
use wb_recorder::wav;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const RATE: u32 = 16_000;

type Setup = (
    TempDir,
    std::path::PathBuf,
    RecorderHandle,
    mpsc::Receiver<Event>,
);

fn input() -> InputInfo {
    InputInfo {
        device_name: "test".into(),
        device_id: "test".into(),
        sample_rate: RATE,
        channels: 1,
        sample_format: "f32".into(),
    }
}

/// Sample `k` of the test signal: a 16-bit counter, exact through f32 and back.
fn sample(k: u64) -> i16 {
    (k % 65_536) as u16 as i16
}

fn setup(name: &str, part_seconds: u32) -> Result<Setup, Box<dyn std::error::Error>> {
    let root = TempDir::new(name)?;
    let spec = SessionSpec::unscheduled(name);
    let dir = session::create_session_dir(root.path(), &spec, now_ist())?;
    let meta = SessionMeta::new(&spec, "test", None, now_ist());
    session::save_meta(&dir, &meta)?;
    let (tx, rx) = mpsc::channel();
    let rec = RecorderHandle::spawn(
        RecorderConfig {
            session_dir: dir.clone(),
            part_seconds,
            meta,
            index: PartsIndex::new(),
        },
        tx,
    )?;
    Ok((root, dir, rec, rx))
}

/// Pushes the counter signal in real time (10 ms blocks) from `start` for `seconds`.
fn feed(prod: &mut impl Producer<Item = f32>, start: u64, seconds: f64) -> u64 {
    feed_blocks(prod, start, seconds, u64::from(RATE / 100))
}

/// `feed` with a chosen block size in frames, one block every 10 ms. A size that does not divide
/// the part length makes some batch straddle a part rollover.
fn feed_blocks(prod: &mut impl Producer<Item = f32>, start: u64, seconds: f64, block: u64) -> u64 {
    let blocks = (seconds * 100.0) as u64;
    let mut k = start;
    for _ in 0..blocks {
        for _ in 0..block {
            let pushed = prod.try_push(f32::from(sample(k)) / 32_768.0).is_ok();
            assert!(pushed, "ring full");
            k += 1;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    k
}

fn all_samples(dir: &std::path::Path) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let index = parts::load(dir)?;
    let mut out = Vec::new();
    for p in &index.parts {
        let (_, s) = wav::read_i16(&dir.join(&p.file))?;
        out.extend(s);
    }
    Ok(out)
}

#[test]
fn parts_hold_every_sample_in_order() -> TestResult {
    let (_root, dir, rec, events) = setup("rec-parts", 1)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let end = feed(&mut prod, 0, 3.5);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert_eq!(report.overrun_samples, 0);
    assert_eq!(report.parts.len(), 4, "{:?}", report.parts);
    assert_eq!(
        report
            .parts
            .iter()
            .map(|p| p.frames)
            .take(3)
            .collect::<Vec<_>>(),
        vec![16_000; 3]
    );
    assert_eq!(report.audio_ms, 3_500);
    let got = all_samples(&dir)?;
    let want: Vec<i16> = (0..end).map(sample).collect();
    assert_eq!(got.len(), want.len());
    assert!(got == want, "samples differ");

    // parts.json equals a rebuild from the files; session closed with a manifest.
    assert_eq!(parts::load(&dir)?, parts::rebuild_from_folder(&dir)?);
    let meta = session::load_meta(&dir)?;
    assert!(meta.closed_at.is_some());
    assert!(meta.events.iter().any(|e| e.kind == "audio_started"));
    assert!(dir.join(session::MANIFEST_FILE).is_file());
    let evs: Vec<Event> = events.try_iter().collect();
    assert_eq!(
        evs.iter()
            .filter(|e| matches!(e, Event::PartClosed(_)))
            .count(),
        4
    );
    assert!(evs.iter().filter(|e| matches!(e, Event::Level(_))).count() >= 60);
    Ok(())
}

#[test]
fn pause_closes_the_part_and_resume_opens_a_new_one() -> TestResult {
    let (_root, dir, rec, _events) = setup("rec-pause", 600)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let k = feed(&mut prod, 0, 1.0);
    std::thread::sleep(Duration::from_millis(50));
    rec.send(RecorderMsg::Pause)?;
    std::thread::sleep(Duration::from_millis(50));
    let k2 = feed(&mut prod, k, 0.5); // discarded while paused
    std::thread::sleep(Duration::from_millis(50));
    rec.send(RecorderMsg::Resume)?;
    std::thread::sleep(Duration::from_millis(20));
    let k3 = feed(&mut prod, k2, 1.0);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert_eq!(report.parts.len(), 2);
    assert_eq!(report.parts[1].start_ms, report.parts[0].duration_ms);
    let got = all_samples(&dir)?;
    // Part 1 is exactly what came before Pause; part 2 ends with what came after Resume.
    let first: Vec<i16> = (0..k).map(sample).collect();
    assert_eq!(&got[..first.len()], &first[..]);
    let after: Vec<i16> = (k2..k3).map(sample).collect();
    assert!(got.ends_with(&after));
    // Nothing fed while paused is in the recording.
    assert_eq!(got.len() as u64, k + (k3 - k2));
    let kinds: Vec<String> = session::load_meta(&dir)?
        .events
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&"paused".to_string()) && kinds.contains(&"resumed".to_string()));
    Ok(())
}

#[test]
fn input_loss_and_reconnect_start_a_new_part() -> TestResult {
    let (_root, dir, rec, events) = setup("rec-reconnect", 600)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let k = feed(&mut prod, 0, 0.5);
    std::thread::sleep(Duration::from_millis(50));
    rec.send(RecorderMsg::InputLost {
        reason: "unplugged".into(),
    })?;
    std::thread::sleep(Duration::from_millis(50));
    let (mut prod2, handle2) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle2))?;
    let k2 = feed(&mut prod2, k, 0.5);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;
    assert_eq!(report.parts.len(), 2);
    assert_eq!(all_samples(&dir)?, (0..k2).map(sample).collect::<Vec<_>>());
    let kinds: Vec<String> = session::load_meta(&dir)?
        .events
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        kinds.contains(&"input_lost".to_string()) && kinds.contains(&"input_restored".to_string()),
        "{kinds:?}"
    );
    assert!(events.try_iter().any(|e| matches!(e, Event::RecorderStatus(s) if s.state == wb_recorder::events::RecState::InputLost)));
    Ok(())
}

/// A panic on another thread (a command handler, here) while recording leaves the recorder
/// writing with no gap: every sample fed during and after the panic is in the parts, in order.
#[test]
fn a_panic_on_another_thread_does_not_stop_the_recorder() -> TestResult {
    let (_root, dir, rec, _events) = setup("rec-panic", 1)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let tx = rec_sender_for_panic(&rec);

    let panicked = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&panicked);
    let feeder = std::thread::spawn(move || feed(&mut prod, 0, 3.0));
    std::thread::sleep(Duration::from_millis(800));
    // An uncaught panic in a plain thread, holding a clone of the recorder's command sender.
    let t = std::thread::spawn(move || {
        let _keep = tx;
        flag.store(true, Ordering::SeqCst);
        panic!("simulated command handler panic");
    });
    assert!(t.join().is_err(), "the thread should have panicked");
    // And a panic caught at a boundary, as the Tauri commands do.
    let caught = std::panic::catch_unwind(|| panic!("simulated panic caught at the boundary"));
    assert!(caught.is_err());

    let end = feeder.join().map_err(|_| "feeder panicked")?;
    std::thread::sleep(Duration::from_millis(100));
    assert!(rec.is_running());
    let report = rec.stop()?;
    assert!(panicked.load(Ordering::SeqCst));
    assert_eq!(report.overrun_samples, 0);
    let got = all_samples(&dir)?;
    assert_eq!(got.len() as u64, end);
    assert!(
        got == (0..end).map(sample).collect::<Vec<_>>(),
        "gap or reordering after the panic"
    );
    Ok(())
}

fn rec_sender_for_panic(rec: &RecorderHandle) -> mpsc::Sender<RecorderMsg> {
    rec.sender()
}

/// A panic inside the recorder thread itself: the open part is closed, recording continues in a
/// new part, and every sample is still there in order.
#[test]
fn a_panic_inside_the_recorder_continues_in_a_new_part() -> TestResult {
    let (_root, dir, rec, _events) = setup("rec-self-panic", 600)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let k = feed(&mut prod, 0, 1.0);
    rec.send(RecorderMsg::InjectPanic(PanicPoint::InMessage))?;
    let end = feed(&mut prod, k, 1.0);
    std::thread::sleep(Duration::from_millis(100));
    assert!(rec.is_running());
    let report = rec.stop()?;
    assert_eq!(report.recorder_restarts, 1);
    assert_eq!(report.parts.len(), 2, "{:?}", report.parts);
    assert_eq!(report.parts[1].start_ms, report.parts[0].duration_ms);
    assert!(all_samples(&dir)? == (0..end).map(sample).collect::<Vec<_>>());
    let kinds: Vec<String> = session::load_meta(&dir)?
        .events
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        kinds.contains(&"recorder_restarted".to_string()),
        "{kinds:?}"
    );
    Ok(())
}

/// A resumed session continues the part numbers and the audio clock where the index ends.
#[test]
fn a_resumed_session_continues_numbering_and_clock() -> TestResult {
    let (_root, dir, rec, _events) = setup("rec-resume", 1)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let k = feed(&mut prod, 0, 1.5);
    std::thread::sleep(Duration::from_millis(100));
    let first = rec.stop()?;
    assert_eq!(first.parts.len(), 2);

    let mut meta = session::load_meta(&dir)?;
    meta.closed_at = None;
    let index = parts::load(&dir)?;
    let (tx, _rx) = mpsc::channel();
    let rec2 = RecorderHandle::spawn(
        RecorderConfig {
            session_dir: dir.clone(),
            part_seconds: 1,
            meta,
            index,
        },
        tx,
    )?;
    let (mut prod2, handle2) = StreamHandle::manual(input(), 20);
    rec2.send(RecorderMsg::Stream(handle2))?;
    let end = feed(&mut prod2, k, 0.5);
    std::thread::sleep(Duration::from_millis(100));
    let second = rec2.stop()?;
    let p = &second.parts;
    assert_eq!(p.len(), 3, "{p:?}");
    assert_eq!(
        (p[2].part, p[2].start_ms, p[2].duration_ms),
        (3, 1_500, 500)
    );
    assert_eq!(second.audio_ms, 2_000);
    assert!(all_samples(&dir)? == (0..end).map(sample).collect::<Vec<_>>());
    assert_eq!(parts::load(&dir)?, parts::rebuild_from_folder(&dir)?);
    // The first stream of a resumed run is not reported as a restored input.
    let kinds: Vec<String> = session::load_meta(&dir)?
        .events
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(!kinds.contains(&"input_restored".to_string()), "{kinds:?}");
    Ok(())
}

/// A panic while writing audio loses nothing: the batch in hand stays with the recorder and the
/// restart finishes it, in order, in the new part. Run for both panic points: before any of the
/// batch is written, and in the middle of a batch that spans a part rollover (some of it already
/// written to the old part, the rest to be written to the new one, none twice).
fn panic_while_writing_loses_nothing(point: PanicPoint, name: &str) -> TestResult {
    // 1-second parts, so a rollover happens every second and some batch spans it.
    let (_root, dir, rec, _events) = setup(name, 1)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    // 150-frame blocks do not divide the 16,000-frame part, so a batch straddles each rollover.
    let k = feed_blocks(&mut prod, 0, 0.5, 150);
    rec.send(RecorderMsg::InjectPanic(point))?;
    // Feed past two rollovers so the injected panic fires (MidBatch waits for a straddling batch).
    let end = feed_blocks(&mut prod, k, 2.5, 150);
    std::thread::sleep(Duration::from_millis(150));
    assert!(rec.is_running());
    let report = rec.stop()?;
    assert_eq!(
        report.recorder_restarts, 1,
        "the injected panic must have fired once"
    );
    assert_eq!(report.dropped_samples, 0);
    assert_eq!(report.overrun_samples, 0);
    let got = all_samples(&dir)?;
    assert_eq!(got.len() as u64, end, "samples missing or duplicated");
    assert!(
        got == (0..end).map(sample).collect::<Vec<_>>(),
        "samples out of order or wrong"
    );
    assert_eq!(parts::load(&dir)?, parts::rebuild_from_folder(&dir)?);
    Ok(())
}

#[test]
fn a_panic_before_writing_a_batch_loses_nothing() -> TestResult {
    panic_while_writing_loses_nothing(PanicPoint::BeforeWrite, "rec-panic-before")
}

#[test]
fn a_panic_in_the_middle_of_a_batch_loses_nothing_and_duplicates_nothing() -> TestResult {
    panic_while_writing_loses_nothing(PanicPoint::MidBatch, "rec-panic-mid")
}

#[test]
fn a_batch_that_keeps_panicking_is_dropped_and_counted_and_recording_goes_on() -> TestResult {
    let (_root, dir, rec, _events) = setup("rec-panic-repeat", 60)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let k = feed_blocks(&mut prod, 0, 0.3, 150);
    std::thread::sleep(Duration::from_millis(200));
    // The next batch panics every time it is tried.
    rec.send(RecorderMsg::InjectPanic(PanicPoint::BeforeWriteRepeated(3)))?;
    let k2 = feed_blocks(&mut prod, k, 0.3, 150);
    std::thread::sleep(Duration::from_millis(400));
    // Audio behind the poisoned batch is recorded.
    let end = feed_blocks(&mut prod, k2, 0.5, 150);
    std::thread::sleep(Duration::from_millis(300));
    assert!(rec.is_running());
    let report = rec.stop()?;
    assert_eq!(
        report.recorder_restarts, 3,
        "three attempts, then the batch is dropped"
    );
    assert!(report.dropped_samples > 0, "the dropped batch is counted");
    let got = all_samples(&dir)?;
    assert_eq!(
        got.len() as u64 + report.dropped_samples,
        end,
        "written plus dropped must equal what was fed"
    );
    assert!(
        got.windows(2).all(|w| w[0] < w[1]),
        "what was written stays in order"
    );
    assert!(
        got.last().copied() == Some(sample(end - 1)),
        "the newest audio is recorded"
    );
    Ok(())
}
