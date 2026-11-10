//! `wb-engine live`: reads 16 kHz mono f32 little-endian PCM from stdin and reports speech
//! start and end as `vad` events on the audio clock. Decoding arrives in Phase 7; this only proves
//! the stream, the VAD model and the event channel work end to end.

use std::io::Read;

use anyhow::Result;
use wb_protocol::{Event, LiveRequest};

use crate::asr::Vad;
use crate::out;

/// How much of the newest audio is looked at each time, in seconds.
const WINDOW_S: usize = 30;
/// How often the VAD runs, in seconds of new audio.
const STEP_S: usize = 1;

pub fn run(req: &LiveRequest) -> Result<()> {
    let mut vad = Vad::load(&req.vad_model.path, (req.threads as usize).clamp(1, 4))?;
    out::emit(&Event::Ready {
        device: req.device,
        device_name: "VAD only".into(),
        threads: req.threads,
        models: Vec::new(),
        load_s: 0.0,
    })?;
    let mut stdin = std::io::stdin().lock();
    let mut buf = vec![0u8; 16_000 * 4 * STEP_S];
    let mut pcm: Vec<f32> = Vec::new();
    // Audio clock: `base` samples were trimmed from the front of `pcm`, so sample i of `pcm` is
    // sample base + i of the stream. `reported_until` is the end of the last speech stretch already
    // announced, in stream samples.
    let mut base = 0usize;
    let mut reported_until = 0usize;
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            let n = stdin.read(&mut buf[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        for b in buf[..filled - filled % 4].as_chunks::<4>().0 {
            pcm.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        }
        let window_start = pcm.len().saturating_sub(16_000 * WINDOW_S);
        let speech = vad.speech(&pcm[window_start..], &req.settings)?;
        for s in speech {
            let a = base + window_start + (s.start_s * 16_000.0) as usize;
            let b = base + window_start + (s.end_s * 16_000.0) as usize;
            // A stretch that ends within half a second of the newest audio may still be going.
            if b + 8_000 < base + pcm.len() && b > reported_until {
                out::emit(&Event::Vad {
                    start_ms: (a / 16) as u64,
                    end_ms: (b / 16) as u64,
                })?;
                reported_until = b;
            }
        }
        if pcm.len() > 16_000 * WINDOW_S * 2 {
            // Keep memory bounded: drop all but the window plus a second.
            let drop = pcm.len() - 16_000 * (WINDOW_S + 1);
            pcm.drain(..drop);
            base += drop;
        }
        if filled < buf.len() {
            break;
        }
    }
    Ok(())
}
