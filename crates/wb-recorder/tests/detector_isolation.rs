//! Phase 2b (R1): the repeated-block and zero-gap detectors run after the audio is written, a
//! panic in one switches only that detector off, and a slow detector is cut off by its time
//! budget. In every case the recording holds every sample that arrived.

use std::sync::mpsc;
use std::time::Duration;

use ringbuf::traits::Producer;
use wb_recorder::capture::StreamHandle;
use wb_recorder::events::Event;
use wb_recorder::parts::{self, AUDIO_DIR, PartsIndex};
use wb_recorder::recorder::{
    DetectorFault, DetectorKind, RecorderConfig, RecorderHandle, RecorderMsg,
};
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

fn setup(name: &str) -> Result<Setup, Box<dyn std::error::Error>> {
    setup_parts(name, 600)
}

fn setup_parts(name: &str, part_seconds: u32) -> Result<Setup, Box<dyn std::error::Error>> {
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

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn noise(frames: usize, seed: u64) -> Vec<i16> {
    (0..frames)
        .map(|f| {
            let r = splitmix(seed ^ (f as u64).wrapping_mul(0x100_0000_01B3));
            ((r >> 32) as u32 % 16_384) as i16 - 8_192
        })
        .collect()
}

/// 4 s of noise with a repeated block (490 frames) at 1.0 s and a zero gap (160 frames) at 3.0 s.
fn audio_with_both() -> Vec<i16> {
    let base = noise(64_000, 21);
    let mut audio = base[..16_000].to_vec();
    audio.extend_from_slice(&base[16_000 - 490..16_000]);
    audio.extend_from_slice(&base[16_000..]);
    let at = 16_000 + 490 + 16_000;
    for s in &mut audio[at..at + 160] {
        *s = 0;
    }
    audio
}

fn feed(prod: &mut impl Producer<Item = f32>, samples: &[i16]) {
    for block in samples.chunks((RATE / 100) as usize) {
        for &s in block {
            assert!(prod.try_push(f32::from(s) / 32_768.0).is_ok(), "ring full");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn recorded(dir: &std::path::Path) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let index = parts::load(dir)?;
    let mut out = Vec::new();
    for p in &index.parts {
        let (_, s) = wav::read_i16(&dir.join(&p.file))?;
        out.extend(s);
    }
    Ok(out)
}

fn events_of(
    dir: &std::path::Path,
    kind: &str,
) -> Result<Vec<session::SessionEvent>, wb_recorder::Error> {
    Ok(session::load_meta(dir)?
        .events
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect())
}

#[test]
fn a_panicking_repeat_detector_is_switched_off_and_the_recording_is_complete() -> TestResult {
    let (_root, dir, rec, _events) = setup("iso-repeat-panic")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    rec.send(RecorderMsg::InjectDetectorFault(DetectorFault::Panic(
        DetectorKind::Repeat,
    )))?;
    let audio = audio_with_both();
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert!(recorded(&dir)? == audio, "every sample is in the recording");
    assert_eq!(report.dropped_samples, 0);
    assert_eq!(
        report.recorder_restarts, 0,
        "a detector panic is not a recorder restart"
    );
    assert_eq!(report.parts.len(), 1, "no new part was opened");
    assert_eq!(
        report.detectors_disabled,
        vec!["repeated_block".to_string()]
    );
    let off = events_of(&dir, "detector_disabled")?;
    assert_eq!(off.len(), 1, "{off:?}");
    assert!(
        off[0]
            .detail
            .clone()
            .unwrap_or_default()
            .contains("detector=repeated_block"),
        "{off:?}"
    );
    // The other detector is untouched; the one that panicked finds nothing from then on.
    assert_eq!(report.zero_gaps, 1);
    assert_eq!(report.repeated_blocks, 0);
    Ok(())
}

#[test]
fn a_panicking_zero_gap_detector_is_switched_off_and_the_recording_is_complete() -> TestResult {
    let (_root, dir, rec, _events) = setup("iso-gap-panic")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    rec.send(RecorderMsg::InjectDetectorFault(DetectorFault::Panic(
        DetectorKind::ZeroGap,
    )))?;
    let audio = audio_with_both();
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert!(recorded(&dir)? == audio, "every sample is in the recording");
    assert_eq!(report.dropped_samples, 0);
    assert_eq!(report.recorder_restarts, 0);
    assert_eq!(report.detectors_disabled, vec!["zero_gap".to_string()]);
    assert_eq!(events_of(&dir, "detector_disabled")?.len(), 1);
    assert_eq!(report.repeated_blocks, 1);
    assert_eq!(report.zero_gaps, 0);
    Ok(())
}

#[test]
fn a_detector_that_panicked_stays_off_after_the_input_reconnects() -> TestResult {
    let (_root, dir, rec, _events) = setup("iso-stay-off")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    rec.send(RecorderMsg::InjectDetectorFault(DetectorFault::Panic(
        DetectorKind::Repeat,
    )))?;
    let first = noise(16_000, 31);
    feed(&mut prod, &first);
    std::thread::sleep(Duration::from_millis(100));
    rec.send(RecorderMsg::InputLost {
        reason: "test".into(),
    })?;
    std::thread::sleep(Duration::from_millis(100));
    let (mut prod2, handle2) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle2))?;
    // The second stream carries a repeated block; the switched-off detector must not see it.
    let base = noise(32_000, 32);
    let mut second = base[..16_000].to_vec();
    second.extend_from_slice(&base[16_000 - 490..16_000]);
    second.extend_from_slice(&base[16_000..]);
    feed(&mut prod2, &second);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    let mut expected = first;
    expected.extend(second);
    assert!(recorded(&dir)? == expected, "both streams are complete");
    assert_eq!(report.repeated_blocks, 0);
    assert_eq!(
        report.detectors_disabled.len(),
        1,
        "one event, not one per stream"
    );
    assert_eq!(events_of(&dir, "detector_disabled")?.len(), 1);
    Ok(())
}

#[test]
fn a_slow_detector_is_cut_off_by_its_budget_and_no_audio_is_lost() -> TestResult {
    let (_root, dir, rec, events) = setup("iso-slow")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    // Every feed of the repeated-block detector takes 200 ms first.
    rec.send(RecorderMsg::InjectDetectorFault(DetectorFault::Sleep(
        DetectorKind::Repeat,
        Duration::from_millis(200),
    )))?;
    let audio = noise(48_000, 41);
    feed(&mut prod, &audio); // the feeder asserts the ring never fills
    std::thread::sleep(Duration::from_millis(400));
    let report = rec.stop()?;

    assert!(recorded(&dir)? == audio, "every sample is in the recording");
    assert_eq!(report.dropped_samples, 0);
    assert_eq!(report.overrun_samples, 0);
    assert_eq!(report.recorder_restarts, 0);
    assert!(
        report.detector_over_budget_blocks > 0,
        "the budget was enforced: {report:?}"
    );
    assert!(report.detector_skipped_ms > 0, "{report:?}");
    assert!(
        report.detector_max_block_us >= 200_000,
        "the slow feed is measured: {}",
        report.detector_max_block_us
    );
    // The ring never came near full (it holds 20 s).
    let mut max_fill = 0.0f32;
    while let Ok(e) = events.try_recv() {
        if let Event::RecorderStatus(s) = e {
            max_fill = max_fill.max(s.ring_max_fill_pct);
        }
    }
    assert!(max_fill < 25.0, "ring fill {max_fill}%");
    Ok(())
}

#[test]
fn the_audio_is_on_disk_while_a_detector_is_still_working_on_it() -> TestResult {
    let (_root, dir, rec, _events) = setup("iso-wav-first")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    rec.send(RecorderMsg::InjectDetectorFault(DetectorFault::Sleep(
        DetectorKind::Repeat,
        Duration::from_secs(2),
    )))?;
    std::thread::sleep(Duration::from_millis(50));
    let block = noise(1_600, 51);
    for &s in &block {
        assert!(prod.try_push(f32::from(s) / 32_768.0).is_ok(), "ring full");
    }
    // The detector now sleeps 2 s on this block. Half a second later the part file already
    // holds the whole block (the first write is flushed at once).
    std::thread::sleep(Duration::from_millis(500));
    let part = std::fs::read_dir(dir.join(AUDIO_DIR))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "wav"))
        .ok_or("no part file yet")?;
    let info = wav::inspect(&part)?;
    assert!(
        info.frames_on_disk >= block.len() as u64,
        "frames on disk {} while the detector was still running",
        info.frames_on_disk
    );
    let report = rec.stop()?;
    assert_eq!(report.dropped_samples, 0);
    assert!(recorded(&dir)? == block);
    Ok(())
}

#[test]
fn zero_gaps_before_across_and_after_a_part_boundary_are_marked_at_their_audio_time() -> TestResult
{
    // Parts of 1 s (16,000 frames). Three gaps of 160 frames (10 ms): one wholly before the
    // boundary, one straddling it, one in the second part.
    let (_root, dir, rec, _events) = setup_parts("iso-rollover", 1)?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let mut audio = noise(40_000, 61);
    let starts = [15_000usize, 15_920, 24_000];
    for at in starts {
        for s in &mut audio[at..at + 160] {
            *s = 0;
        }
    }
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert_eq!(report.parts.len(), 3, "40,000 frames in parts of 16,000");
    assert!(
        recorded(&dir)? == audio,
        "every sample, across the rollover"
    );
    assert_eq!(report.zero_gaps, 3, "{report:?}");
    let marked = events_of(&dir, "zero_gap")?;
    assert_eq!(marked.len(), 3, "{marked:?}");
    for (m, at) in marked.iter().zip(starts) {
        let want = at as f64 * 1000.0 / f64::from(RATE);
        assert!(
            (m.audio_ms as f64 - want).abs() <= 3.0,
            "gap at frame {at}: expected {want} ms, marked at {} ms",
            m.audio_ms
        );
    }
    Ok(())
}
