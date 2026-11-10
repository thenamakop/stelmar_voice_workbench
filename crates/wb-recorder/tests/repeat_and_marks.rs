//! The recorder thread marks what the capture callbacks and the audio itself show: a repeated
//! block in the recorded audio, a driver xrun with its place on the audio clock, and the
//! position-check findings. Fed by a manual stream instead of a device.

use std::sync::mpsc;
use std::time::Duration;

use ringbuf::traits::Producer;
use wb_recorder::capture::{PositionKind, PositionMark, StreamHandle, XrunMark};
use wb_recorder::events::Event;
use wb_recorder::parts::{self, PartsIndex};
use wb_recorder::recorder::{RecorderConfig, RecorderHandle, RecorderMsg};
use wb_recorder::session::{self, InputInfo, SessionMeta, SessionSpec};
use wb_recorder::testutil::TempDir;
use wb_recorder::util::now_ist;
use wb_recorder::wav;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const RATE: u32 = 16_000;

fn input() -> InputInfo {
    InputInfo {
        device_name: "test".into(),
        device_id: "test".into(),
        sample_rate: RATE,
        channels: 1,
        sample_format: "f32".into(),
    }
}

type Setup = (
    TempDir,
    std::path::PathBuf,
    RecorderHandle,
    mpsc::Receiver<Event>,
);

fn setup(name: &str) -> Result<Setup, Box<dyn std::error::Error>> {
    let root = TempDir::new(name)?;
    let spec = SessionSpec::unscheduled(name);
    let dir = session::create_session_dir(root.path(), &spec, now_ist())?;
    let meta = SessionMeta::new(&spec, "test", None, now_ist());
    session::save_meta(&dir, &meta)?;
    let (tx, rx) = mpsc::channel();
    let rec = RecorderHandle::spawn(
        RecorderConfig {
            session_dir: dir.clone(),
            part_seconds: 600,
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

/// 16-bit-step noise at about -12 dBFS peak, exact through f32 and back.
fn noise(frames: usize, seed: u64) -> Vec<i16> {
    (0..frames)
        .map(|f| {
            let r = splitmix(seed ^ (f as u64).wrapping_mul(0x100_0000_01B3));
            ((r >> 32) as u32 % 16_384) as i16 - 8_192
        })
        .collect()
}

/// Pushes `samples` in 10 ms blocks, one block every 10 ms, like a device.
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
fn a_repeated_block_in_the_audio_is_marked_at_its_audio_time_and_recorded_as_received() -> TestResult
{
    let (_root, dir, rec, _events) = setup("repeat-marked")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    // 4 s of noise with the 490 frames before 2.0 s played twice (a 30.6 ms block at 16 kHz).
    let base = noise(64_000, 1);
    let mut audio = base[..32_000].to_vec();
    audio.extend_from_slice(&base[32_000 - 490..32_000]);
    audio.extend_from_slice(&base[32_000..]);
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert_eq!(report.repeated_blocks, 1);
    assert_eq!(report.dropped_samples, 0);
    // The recording holds exactly what arrived, repeat included.
    assert!(
        recorded(&dir)? == audio,
        "the recording differs from the input"
    );

    let marked = events_of(&dir, "repeated_block")?;
    assert_eq!(marked.len(), 1, "{marked:?}");
    let at = marked[0].audio_ms as i64;
    assert!((at - 2_000).abs() <= 2, "audio_ms {at}");
    let detail = marked[0].detail.clone().unwrap_or_default();
    assert!(detail.contains("len_frames=490 lag_frames=490"), "{detail}");
    let log = std::fs::read_to_string(dir.join("session.log"))?;
    assert!(log.contains("repeated_block audio_ms="), "{log}");
    Ok(())
}

#[test]
fn clean_noise_and_silence_are_not_marked() -> TestResult {
    let (_root, dir, rec, _events) = setup("repeat-clean")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let mut audio = noise(32_000, 2);
    audio.extend(std::iter::repeat_n(0i16, 16_000));
    audio.extend(noise(16_000, 3));
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;
    assert_eq!(report.repeated_blocks, 0);
    assert!(events_of(&dir, "repeated_block")?.is_empty());
    Ok(())
}

#[test]
fn a_repeat_at_the_end_of_the_recording_is_flushed_at_stop() -> TestResult {
    let (_root, dir, rec, _events) = setup("repeat-at-stop")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    let base = noise(32_000, 4);
    let mut audio = base.clone();
    audio.extend_from_slice(&base[32_000 - 400..]);
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;
    assert_eq!(report.repeated_blocks, 1);
    let marked = events_of(&dir, "repeated_block")?;
    assert_eq!(marked.len(), 1);
    assert!(
        (marked[0].audio_ms as i64 - 2_000).abs() <= 2,
        "{:?}",
        marked[0]
    );
    Ok(())
}

#[test]
fn an_xrun_is_logged_with_its_callback_count_audio_position_and_qpc() -> TestResult {
    let (_root, dir, rec, _events) = setup("xrun-logged")?;
    let (mut p, handle) = StreamHandle::manual_with_marks(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    // One second in; the driver reported an xrun after 8,000 frames and 50 callbacks.
    feed(&mut p.audio, &noise(16_000, 5));
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        p.xruns
            .try_push(XrunMark {
                callbacks: 50,
                frames: 8_000,
                qpc: 123_456_789,
            })
            .is_ok()
    );
    std::thread::sleep(Duration::from_millis(150));
    let report = rec.stop()?;
    assert_eq!(report.dropped_samples, 0);

    let gaps = events_of(&dir, "gap")?;
    let xrun = gaps
        .iter()
        .find(|e| {
            e.detail
                .as_deref()
                .is_some_and(|d| d.starts_with("cause=xrun"))
        })
        .ok_or("no xrun gap event")?;
    let detail = xrun.detail.clone().unwrap_or_default();
    assert!(detail.contains("n=1 callback=50 frames=8000"), "{detail}");
    assert!(detail.contains("qpc=123456789"), "{detail}");
    // 8,000 frames at 16 kHz is 500 ms into the audio.
    assert!((xrun.audio_ms as i64 - 500).abs() <= 20, "{xrun:?}");
    let log = std::fs::read_to_string(dir.join("session.log"))?;
    assert!(log.contains("cause=xrun n=1 callback=50"), "{log}");
    Ok(())
}

#[test]
fn position_findings_reach_the_log_and_only_breaks_reach_session_json() -> TestResult {
    let (_root, dir, rec, _events) = setup("position-logged")?;
    let (mut p, handle) = StreamHandle::manual_with_marks(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    feed(&mut p.audio, &noise(16_000, 6));
    std::thread::sleep(Duration::from_millis(100));
    let mark = |kind, callback| PositionMark {
        kind,
        callback,
        frames: 8_000,
        ts_ns: 500_000_000,
        dev_ns: 4_000_000,
        pair_diff_ns: 4_000_000,
    };
    assert!(
        p.positions
            .try_push(mark(PositionKind::Outlier, 10))
            .is_ok()
    );
    assert!(
        p.positions
            .try_push(mark(PositionKind::PairFlag, 10))
            .is_ok()
    );
    assert!(p.positions.try_push(mark(PositionKind::Break, 11)).is_ok());
    std::thread::sleep(Duration::from_millis(150));
    rec.stop()?;

    let log = std::fs::read_to_string(dir.join("session.log"))?;
    assert!(log.contains("timestamp_outlier callback=10"), "{log}");
    assert!(log.contains("position_pair_flag callback=10"), "{log}");
    assert!(log.contains("position_break callback=11"), "{log}");
    let gaps = events_of(&dir, "gap")?;
    let in_json: Vec<_> = gaps
        .iter()
        .filter_map(|e| e.detail.as_deref())
        .filter(|d| d.contains("position_"))
        .collect();
    assert_eq!(in_json.len(), 1, "{in_json:?}");
    assert!(in_json[0].contains("position_break"));
    Ok(())
}

#[test]
fn a_zero_gap_in_the_audio_is_marked_at_its_audio_time_and_recorded_as_received() -> TestResult {
    let (_root, dir, rec, _events) = setup("zero-gap-marked")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    // 4 s of noise with 160 frames (10 ms at 16 kHz) of exact zeros at 2.0 s: the shape of the
    // shared upstream breaks in the 10 Nov 2026 gate recordings.
    let mut audio = noise(64_000, 7);
    for s in &mut audio[32_000..32_160] {
        *s = 0;
    }
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;

    assert_eq!(report.zero_gaps, 1);
    assert_eq!(report.repeated_blocks, 0);
    assert_eq!(report.dropped_samples, 0);
    assert!(
        recorded(&dir)? == audio,
        "the recording differs from the input"
    );
    let marked = events_of(&dir, "zero_gap")?;
    assert_eq!(marked.len(), 1, "{marked:?}");
    assert!(
        (marked[0].audio_ms as i64 - 2_000).abs() <= 2,
        "{:?}",
        marked[0]
    );
    let detail = marked[0].detail.clone().unwrap_or_default();
    assert!(detail.contains("len_frames=160"), "{detail}");
    let log = std::fs::read_to_string(dir.join("session.log"))?;
    assert!(log.contains("zero_gap audio_ms="), "{log}");
    Ok(())
}

#[test]
fn silence_between_two_recordings_of_noise_is_not_a_zero_gap() -> TestResult {
    let (_root, dir, rec, _events) = setup("zero-gap-silence")?;
    let (mut prod, handle) = StreamHandle::manual(input(), 20);
    rec.send(RecorderMsg::Stream(handle))?;
    // A long mute (over 50 ms) between two stretches of noise is not a dropout.
    let mut audio = noise(32_000, 8);
    audio.extend(std::iter::repeat_n(0i16, 8_000));
    audio.extend(noise(16_000, 9));
    feed(&mut prod, &audio);
    std::thread::sleep(Duration::from_millis(100));
    let report = rec.stop()?;
    assert_eq!(report.zero_gaps, 0);
    assert!(events_of(&dir, "zero_gap")?.is_empty());
    Ok(())
}
