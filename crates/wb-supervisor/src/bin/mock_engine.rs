//! A stand-in for `wb-engine batch` that the supervisor tests drive. It speaks the same protocol
//! (hello, ready, segments in audio order, progress, done) and fails on demand.
//!
//! Environment:
//!   MOCK_CHUNKS   chunks in the job (default 20); chunk i spans [i*1000, (i+1)*1000) ms
//!   MOCK_PLAN     what each launch does, comma separated, the last repeats: ok | die:K | hang:K |
//!                 fatal:CODE | die_between:K | slow:S | slow_silent:S
//!                 (die and hang act after K chunks of that launch; die_between emits the FIRST half of
//!                 chunk K and then dies, before the chunk's Progress; slow works for S seconds on the
//!                 first chunk sending a heartbeat every 200 ms, slow_silent for S seconds sending nothing)
//!   MOCK_SKIP     chunk index that gives a warning and no segment (a chunk too short to decode)
//!   MOCK_SPLIT    1: every chunk is two segments (a split chunk: two halves), then one Progress
//!   MOCK_COUNTER  a file that counts launches
//!   MOCK_DELAY_MS pause between chunks (default 2)

use std::io::Write;

use wb_protocol::{Event, Hello, Request, SegmentEvent};

fn emit(e: &Event) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", e.to_line().unwrap_or_default());
    let _ = out.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .iter()
        .position(|a| a == "--request")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default();
    let resume = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Request>(&t).ok())
    {
        Some(Request::Batch(b)) => b.resume_from_ms,
        _ => {
            eprintln!("mock engine: no valid batch request");
            std::process::exit(2);
        }
    };
    let chunks: u64 = std::env::var("MOCK_CHUNKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let delay: u64 = std::env::var("MOCK_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let launch: usize = match std::env::var("MOCK_COUNTER") {
        Ok(p) => {
            let n = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0);
            let _ = std::fs::write(&p, format!("{}", n + 1));
            n
        }
        Err(_) => 0,
    };
    let plan = std::env::var("MOCK_PLAN").unwrap_or_else(|_| "ok".into());
    let steps: Vec<&str> = plan.split(',').collect();
    let step = steps.get(launch).or(steps.last()).copied().unwrap_or("ok");

    emit(&Event::Hello(Hello::new("mock")));
    if let Some(code) = step.strip_prefix("fatal:") {
        emit(&Event::Error {
            code: code.into(),
            message: format!("mock fatal error {code}"),
            fatal: true,
        });
        std::process::exit(4);
    }
    emit(&Event::Ready {
        device: wb_protocol::Device::Cpu,
        device_name: "mock".into(),
        threads: 1,
        models: vec!["mock".into()],
        load_s: 0.0,
    });
    let (kind, after): (&str, u64) = match step.split_once(':') {
        Some((k, n)) => (k, n.parse().unwrap_or(0)),
        None => (step, u64::MAX),
    };
    let split = std::env::var("MOCK_SPLIT").is_ok_and(|v| v == "1");
    let segment = |i: u64, start: u64, end: u64, label: &str| {
        emit(&Event::Segment(SegmentEvent {
            unit: 0,
            chunk: i as u32,
            start_ms: start,
            end_ms: end,
            text: format!("segment {i}{label} नमस्ते"),
            lang: "hi".into(),
            lang_prob: 1.0,
            mixed: false,
            dir: "ltr".into(),
            avg_logprob: -0.2,
            no_speech_prob: 0.01,
            compression_ratio: 1.1,
            model: "mock".into(),
            decoded_both_ways: false,
            detection: vec![],
            flags: vec![],
            truncated: false,
            hint_text: None,
        }));
    };
    let mut emitted = 0u64;
    let mut worked = false;
    for i in 0..chunks {
        let (start, end) = (i * 1000, (i + 1) * 1000);
        if end <= resume {
            continue;
        }
        if emitted == after && kind != "slow" && kind != "slow_silent" {
            match kind {
                "die" => std::process::exit(1),
                "hang" => loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                },
                "die_between" => {
                    segment(i, start, start + 500, "a");
                    std::process::exit(1);
                }
                _ => {}
            }
        }
        if !worked && (kind == "slow" || kind == "slow_silent") {
            worked = true;
            let until = std::time::Instant::now() + std::time::Duration::from_secs(after);
            while std::time::Instant::now() < until {
                if kind == "slow" {
                    emit(&Event::Heartbeat {
                        unit: 0,
                        phase: "decode".into(),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(delay));
        if std::env::var("MOCK_SKIP")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            == Some(i)
        {
            // A chunk the engine could not decode (too short): a warning, no segment, then the chunk is closed.
            emit(&Event::Warning {
                code: "chunk_too_short".into(),
                message: format!("chunk {i} holds under 100 ms of audio and was not decoded"),
            });
        } else if split {
            segment(i, start, start + 500, "a");
            segment(i, start + 500, end, "b");
        } else {
            segment(i, start, end, "");
        }
        emit(&Event::Progress {
            unit: 0,
            committed_until_ms: end,
            unit_end_ms: chunks * 1000,
        });
        emitted += 1;
    }
    emit(&Event::Done(wb_protocol::DoneSummary {
        segments: emitted as u32,
        chunks: emitted as u32,
        audio_ms: chunks * 1000,
        truncated_chunks: 0,
        elapsed_s: 0.0,
        retried_chunks: 0,
        unclear_chunks: 0,
        low_coverage_chunks: 0,
    }));
}
