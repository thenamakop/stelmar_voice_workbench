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

// ---- Write failures ---------------------------------------------------------------------------
//
// A real write failure part-way through a part: a second handle on the open part file takes a
// shared lock, and Windows then refuses the recorder's writes to it with a lock violation (the
// same kind of error as a full disk: the write call fails) while reads still work. Nothing in
// the recorder is replaced or hooked for these tests.

/// Whole samples of a part file read from its length, ignoring a header that is out of date.
fn raw_samples(path: &std::path::Path) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let info = wav::inspect(path)?;
    let bytes = std::fs::read(path)?;
    let start = info.data_offset as usize;
    let end = start + info.frames_on_disk as usize * usize::from(info.block_align);
    Ok(bytes[start..end]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect())
}

/// Every sample in every part file of the session folder, in part order, read from file lengths.
fn raw_all_samples(dir: &std::path::Path) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for n in parts::part_numbers_on_disk(dir)? {
        out.extend(raw_samples(
            &dir.join("audio").join(parts::part_file_name(n)),
        )?);
    }
    Ok(out)
}

/// Feeds audio until the recorder has flushed part 1 once (its header then covers about a
/// second) and 0.6 s more, so some audio is in the file beyond what its header counts but the
/// next one-second flush has not run. Returns the next sample index.
fn feed_until_between_flushes(
    prod: &mut impl Producer<Item = f32>,
    part: &std::path::Path,
    start: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut k = feed(prod, start, 1.1);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !wav::inspect(part).is_ok_and(|i| i.header_data_len >= 30_000) {
        if std::time::Instant::now() > deadline {
            return Err("the first flush never happened".into());
        }
        k = feed(prod, k, 0.1);
    }
    Ok(feed(prod, k, 0.6))
}

fn expected(len: u64) -> Vec<i16> {
    (0..len).map(sample).collect()
}

/// Ticks the stream's callback counter like a live device, so the five-second no-audio watchdog
/// does not report a manual stream as lost during these long tests.
struct Ticker {
    on: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Ticker {
    fn start(handle: &StreamHandle) -> Self {
        let counters = Arc::clone(&handle.counters);
        let on = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&on);
        let join = std::thread::spawn(move || {
            while flag.load(Ordering::Relaxed) {
                counters.callbacks.fetch_add(1, Ordering::Relaxed);
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

fn stop_with_timeout(
    rec: RecorderHandle,
) -> Result<wb_recorder::recorder::StopReport, Box<dyn std::error::Error>> {
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(rec.stop());
    });
    Ok(done_rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "Stop hung after a write failure")??)
}

/// Error notices and whether any message contains `needle`.
fn drain_errors(events: &mpsc::Receiver<Event>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let Event::Notice(n) = ev {
            if n.level == wb_recorder::events::NoticeLevel::Error {
                out.push(n.message);
            }
        }
    }
    out
}

/// The disk fails part-way through part 1 and works again for part 2. Every sample captured is
/// on disk exactly once, in order; nothing is counted as dropped; parts.json, the mirror and a
/// rebuild from the files agree; part 1 is kept and marked needs_repair (its header cannot be
/// rewritten while the fault lasts), and recovery repairs it.
#[test]
#[allow(clippy::incompatible_msrv)] // File::lock_shared needs Rust 1.89; the test toolchain is newer.
fn a_write_failure_part_way_loses_nothing_and_duplicates_nothing() -> TestResult {
    use std::fs::OpenOptions;
    use wb_recorder::mirror::MirrorHandle;
    use wb_recorder::recovery;

    let (root, dir, rec, events) = setup("rec-write-fail", 600)?;
    let backup = root.path().join("backup");
    let (mirror_tx, _mirror_rx) = mpsc::channel();
    let mut mirror = MirrorHandle::spawn(dir.clone(), Some(backup.clone()), mirror_tx)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    let ticker = Ticker::start(&handle);
    rec.send(RecorderMsg::Stream(handle))?;

    // Audio until part 1 holds data beyond what its header counts, mid-way between two flushes.
    let first_part = dir.join("audio").join(parts::part_file_name(1));
    let k1 = feed_until_between_flushes(&mut prod, &first_part, 0)?;
    assert!(first_part.exists(), "the first part is open");

    // The disk "fails": writes to part 1 are refused from now on (reads still work).
    let blocker = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&first_part)?;
    blocker.lock_shared()?;
    let k2 = feed(&mut prod, k1, 2.0);
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(
        rec.is_running(),
        "the recorder thread survives a failed write"
    );

    // Part 2 takes the audio that follows (and the audio that part 1 could not take).
    let end = feed(&mut prod, k2, 2.0);
    std::thread::sleep(Duration::from_millis(300));
    let report = stop_with_timeout(rec)?;
    let verify = mirror.final_sync();
    mirror.stop();
    drop(ticker);

    // Reported.
    let errors = drain_errors(&events);
    assert!(
        !errors.is_empty(),
        "the failed write must reach the UI as an error"
    );
    let meta = session::load_meta(&dir)?;
    assert!(
        meta.events.iter().any(|e| e.kind == "write_error"),
        "session.json must log the write error"
    );

    // Frames on disk (all parts) plus dropped samples equal the frames fed; the lock only
    // affects part 1, so part 2 takes everything and nothing is dropped.
    let index = parts::load(&dir)?;
    let on_disk: u64 = index.parts.iter().map(|p| p.frames).sum();
    assert_eq!(
        on_disk + report.dropped_samples,
        end,
        "frames on disk + dropped must equal frames fed; parts {:?}",
        index.parts
    );
    assert_eq!(report.dropped_samples, 0, "the next part took everything");

    // Not one sample lost or repeated, including around the cut between the parts.
    let got = raw_all_samples(&dir)?;
    assert_eq!(got.len() as u64, end);
    assert!(
        got == expected(end),
        "samples missing, repeated or reordered"
    );

    // Part 1 is kept, marked needs_repair; parts.json matches a rebuild from the files.
    assert!(index.parts.len() >= 2, "{:?}", index.parts);
    assert!(
        index.parts[0].needs_repair,
        "part 1 closed with an unrepaired header"
    );
    assert!(!index.parts[1].needs_repair);
    assert_eq!(index, parts::rebuild_from_folder(&dir)?);

    // The mirror holds the same bytes, including part 1.
    assert!(verify.all_match, "mirror differs: {:?}", verify.message);
    let mirror_part = backup.join("audio").join(parts::part_file_name(1));
    assert!(mirror_part.exists(), "the mirror has no copy of part 1");
    assert_eq!(std::fs::read(&mirror_part)?, std::fs::read(&first_part)?);

    // The fault clears. Recovery rewrites part 1's header; everything plays and matches.
    blocker.unlock()?;
    drop(blocker);
    assert!(!wav::inspect(&first_part)?.header_matches_length());
    recovery::repair(&dir)?;
    assert!(wav::inspect(&first_part)?.header_matches_length());
    let fixed = parts::load(&dir)?;
    assert!(fixed.parts.iter().all(|p| !p.needs_repair));
    assert_eq!(fixed, parts::rebuild_from_folder(&dir)?);
    assert!(
        all_samples(&dir)? == expected(end),
        "samples differ after recovery"
    );
    Ok(())
}

/// The part after the failed one cannot be opened either (a stray file has its number). What the
/// failed part owed the recording cannot be written, so it is counted, not lost silently; what
/// was already on disk stays; recording goes on in the part after that.
#[test]
#[allow(clippy::incompatible_msrv)] // File::lock_shared needs Rust 1.89; the test toolchain is newer.
fn when_the_next_part_fails_too_the_loss_is_counted_not_silent() -> TestResult {
    use std::fs::OpenOptions;
    use wb_recorder::mirror::MirrorHandle;

    let (root, dir, rec, events) = setup("rec-write-fail-twice", 600)?;
    let backup = root.path().join("backup");
    let (mirror_tx, _mirror_rx) = mpsc::channel();
    let mut mirror = MirrorHandle::spawn(dir.clone(), Some(backup), mirror_tx)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    let ticker = Ticker::start(&handle);
    rec.send(RecorderMsg::Stream(handle))?;

    let audio = dir.join("audio");
    let first_part = audio.join(parts::part_file_name(1));
    let k1 = feed_until_between_flushes(&mut prod, &first_part, 0)?;
    // Part 2 cannot be created: something already has its name.
    let stray = audio.join(parts::part_file_name(2));
    std::fs::write(&stray, b"not a part")?;
    let blocker = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&first_part)?;
    blocker.lock_shared()?;
    let k2 = feed(&mut prod, k1, 2.0);
    std::thread::sleep(Duration::from_millis(2_500));
    let end = feed(&mut prod, k2, 2.0);
    std::thread::sleep(Duration::from_millis(300));
    let report = stop_with_timeout(rec)?;
    let verify = mirror.final_sync();
    mirror.stop();
    drop(ticker);
    blocker.unlock()?;
    drop(blocker);
    std::fs::remove_file(&stray)?;

    let errors = drain_errors(&events);
    assert!(
        errors.iter().any(|m| m.contains("Could not open part")),
        "the second failure must be reported: {errors:?}"
    );
    let meta = session::load_meta(&dir)?;
    assert!(meta.events.iter().any(|e| e.kind == "write_error"));

    // Counted: frames on disk + dropped samples == frames fed. Some loss is expected here and
    // it is bounded by what a failed part holds (about a second) plus one batch.
    let index = parts::load(&dir)?;
    let on_disk: u64 = index.parts.iter().map(|p| p.frames).sum();
    assert_eq!(
        on_disk + report.dropped_samples,
        end,
        "parts {:?}",
        index.parts
    );
    assert!(report.dropped_samples > 0, "the loss must be counted");
    assert!(
        report.dropped_samples <= u64::from(RATE) * 2,
        "dropped {} is more than a failed part and a batch can hold",
        report.dropped_samples
    );

    // What is on disk is the start of the recording, then the end of it, each intact: part 1
    // holds the start, the last part the audio from after the lost stretch.
    let got = raw_all_samples(&dir)?;
    let want = expected(end);
    let n1 = index.parts[0].frames as usize;
    assert!(
        got[..n1] == want[..n1],
        "audio before the failure was damaged"
    );
    let tail = got.len() - n1;
    assert!(
        got[n1..] == want[want.len() - tail..],
        "audio after the lost stretch was damaged"
    );
    assert_eq!(got.last().copied(), Some(sample(end - 1)));
    assert_eq!(index, parts::rebuild_from_folder(&dir)?);
    assert!(verify.all_match, "mirror differs: {:?}", verify.message);
    Ok(())
}
