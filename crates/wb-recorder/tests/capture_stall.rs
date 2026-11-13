//! The capture thread kept away from a real device, end to end: the real WASAPI stream (opened the
//! way a session opens it, with its 1,000 ms device buffer) feeds the real recorder, a known tone
//! plays into the virtual cable, and the data callback is made to stall for 300 ms, 700 ms and
//! longer than the granted buffer.
//!
//! Needs the VB-CABLE pair on the machine and is switched on explicitly, because it plays a tone
//! into the cable and must not run while a gate does:
//!
//!   $env:WB_CAPTURE_STALL_TEST = "1"
//!   cargo test -p wb-recorder --features test-hooks --test capture_stall -- --nocapture
//!
//! Without the variable (or without the cable) the test prints why it did nothing and passes.
//! `WB_CAPTURE_STALL_IN` and `WB_CAPTURE_STALL_OUT` change the device names ("CABLE Output" and
//! "CABLE Input").

#![cfg(windows)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use wb_recorder::capture::{self, StreamCounters};
use wb_recorder::parts::PartsIndex;
use wb_recorder::recorder::{RecorderConfig, RecorderHandle, RecorderMsg, StopReport};
use wb_recorder::session::{self, SessionMeta, SessionSpec};
use wb_recorder::testutil::TempDir;
use wb_recorder::util::now_ist;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn name_from_env(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// A quiet noise played into the cable so that the capture side has a signal and a clock.
fn play_noise_into(output_name: &str) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let devices = host.output_devices().map_err(|e| e.to_string())?;
    let device = devices
        .into_iter()
        .find(|d| d.to_string().contains(output_name))
        .ok_or_else(|| format!("no output device named like {output_name}"))?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err("the cable's output format is not f32".into());
    }
    let mut state = 0x1234_5678_u32;
    let stream = device
        .build_output_stream::<f32, _, _>(
            config.config(),
            move |data: &mut [f32], _| {
                for s in data.iter_mut() {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    *s = (state as f32 / u32::MAX as f32 - 0.5) * 0.1;
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

struct Run {
    report: StopReport,
    counters: Arc<StreamCounters>,
    granted_ms: f64,
    elapsed_ms: f64,
    log: String,
    session_json: String,
}

/// Records from the real input for `before_ms`, stalls the data callback for `pause_ms` (0: no
/// stall), records `after_ms` more and stops. Returns what the recorder and the counters say.
fn run(
    input_name: &str,
    pause_ms: u64,
    before_ms: u64,
    after_ms: u64,
) -> Result<Run, Box<dyn std::error::Error>> {
    let root = TempDir::new("capture-stall")?;
    let spec = SessionSpec::unscheduled("capture-stall");
    let dir = session::create_session_dir(root.path(), &spec, now_ist())?;
    let meta = SessionMeta::new(&spec, "test", None, now_ist());
    session::save_meta(&dir, &meta)?;
    let (tx, _events) = mpsc::channel();
    let rec = RecorderHandle::spawn(
        RecorderConfig {
            session_dir: dir.clone(),
            part_seconds: 600,
            meta,
            index: PartsIndex::new(),
        },
        tx,
    )?;
    let (stream, handle) = capture::open_for_test(input_name)?;
    let counters = Arc::clone(&handle.counters);
    let granted_ms = handle
        .buffer
        .granted_ms
        .ok_or("the granted buffer could not be read")?;
    let started = Instant::now();
    rec.send(RecorderMsg::Stream(handle))?;
    std::thread::sleep(Duration::from_millis(before_ms));
    if pause_ms > 0 {
        counters.inject_pause_ms.store(pause_ms, Ordering::Relaxed);
    }
    std::thread::sleep(Duration::from_millis(pause_ms + after_ms));
    drop(stream);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let report = rec.stop()?;
    let log = std::fs::read_to_string(dir.join("session.log")).unwrap_or_default();
    let session_json = std::fs::read_to_string(dir.join("session.json")).unwrap_or_default();
    Ok(Run {
        report,
        counters,
        granted_ms,
        elapsed_ms,
        log,
        session_json,
    })
}

#[test]
fn a_capture_thread_stall_inside_the_granted_buffer_loses_nothing_and_one_beyond_it_is_logged()
-> TestResult {
    if std::env::var("WB_CAPTURE_STALL_TEST").is_err() {
        println!(
            "capture_stall: skipped (set WB_CAPTURE_STALL_TEST=1 to play a tone into the cable)"
        );
        return Ok(());
    }
    let input = name_from_env("WB_CAPTURE_STALL_IN", "CABLE Output");
    let output = name_from_env("WB_CAPTURE_STALL_OUT", "CABLE Input");
    let _tone = match play_noise_into(&output) {
        Ok(s) => s,
        Err(why) => {
            println!("capture_stall: skipped ({why})");
            return Ok(());
        }
    };
    std::thread::sleep(Duration::from_millis(500));

    // Baseline: no stall. The callback period, the buffer asked for and the buffer granted.
    let base = run(&input, 0, 1_000, 4_000)?;
    println!("baseline: {:?}", base.report.capture_buffers);
    println!(
        "baseline: callback period mean {} ms, shortest {} ms, longest gap {:.1} ms, audio {} ms of {:.0} ms wall",
        base.report
            .capture_callback_period_ms
            .map_or_else(|| "n/a".to_string(), |p| format!("{p:.2}")),
        base.report
            .capture_callback_period_min_ms
            .map_or_else(|| "n/a".to_string(), |p| format!("{p:.3}")),
        base.report.capture_max_callback_gap_ms,
        base.report.audio_ms,
        base.elapsed_ms
    );
    assert_eq!(base.report.xruns, 0);
    assert!(
        base.granted_ms >= 999.0,
        "the 1,000 ms request was granted as {} ms",
        base.granted_ms
    );
    assert!(base.report.capture_buffers[0].starts_with("requested 1000 ms"));
    let period = base
        .report
        .capture_callback_period_ms
        .ok_or("no callback period")?;
    assert!(
        (8.0..=12.0).contains(&period),
        "the callback period stays about 10 ms in shared mode, measured {period:.2} ms"
    );
    assert!(base.log.contains("capture_buffer") && base.session_json.contains("capture_buffer"));

    // Stalls inside the buffer: nothing may be lost, and the margin has to say how close it came.
    for pause in [300_u64, 700] {
        let r = run(&input, pause, 1_500, 2_500)?;
        let gap = r.report.capture_max_callback_gap_ms;
        let margin = r.report.capture_margin_ms.ok_or("no margin")?;
        println!(
            "pause {pause} ms: longest callback gap {gap:.1} ms, granted {:.0} ms, margin {margin:.1} ms, audio {} ms of {:.0} ms wall, xruns {}",
            r.granted_ms, r.report.audio_ms, r.elapsed_ms, r.report.xruns
        );
        assert!(
            gap >= pause as f64 * 0.95 && gap < pause as f64 + 300.0,
            "the stall shows as the longest gap: {gap:.1} ms for a {pause} ms pause"
        );
        assert_eq!(r.report.xruns, 0, "no driver xrun for a {pause} ms stall");
        assert_eq!(r.counters.overrun_samples.load(Ordering::Relaxed), 0);
        assert_eq!(r.report.dropped_samples, 0);
        assert_eq!(
            r.report.position_breaks, 0,
            "the device clock shows no break"
        );
        assert!(margin > 0.0, "margin {margin:.1} ms");
        // The recording holds the whole time, to within a few callbacks.
        assert!(
            r.report.audio_ms as f64 >= r.elapsed_ms - 400.0,
            "recorded {} ms of {:.0} ms",
            r.report.audio_ms,
            r.elapsed_ms
        );
        assert!(r.report.capture_gaps_ge_250ms >= 1);
        assert_eq!(r.report.capture_gaps_ge_500ms >= 1, pause >= 500);
        assert!(
            r.log.contains("capture_stall"),
            "the near miss is in session.log"
        );
        assert!(r.session_json.contains("capture_max_callback_gap_ms"));
    }

    // A stall longer than the granted buffer: the audio engine overwrites, the driver flags the
    // discontinuity, and the recorder says so in session.log and session.json.
    let probe_run = run(&input, 0, 200, 200)?;
    let beyond = probe_run.granted_ms as u64 + 600;
    let r = run(&input, beyond, 1_500, 2_500)?;
    let gap = r.report.capture_max_callback_gap_ms;
    let margin = r.report.capture_margin_ms.ok_or("no margin")?;
    println!(
        "pause {beyond} ms (granted {:.0} ms): longest gap {gap:.1} ms, margin {margin:.1} ms, audio {} ms of {:.0} ms wall, xruns {}",
        r.granted_ms, r.report.audio_ms, r.elapsed_ms, r.report.xruns
    );
    assert!(
        r.report.xruns >= 1,
        "a stall beyond the buffer must be an xrun"
    );
    assert!(margin < 0.0, "margin {margin:.1} ms");
    assert!(
        r.log.contains("cause=xrun"),
        "the xrun is in session.log:\n{}",
        r.log
    );
    assert!(
        r.session_json.contains("xrun"),
        "the xrun is in session.json"
    );
    assert!(
        (r.report.audio_ms as f64) < r.elapsed_ms - 100.0,
        "the audio the engine overwrote is missing from the recording ({} of {:.0} ms)",
        r.report.audio_ms,
        r.elapsed_ms
    );
    Ok(())
}
