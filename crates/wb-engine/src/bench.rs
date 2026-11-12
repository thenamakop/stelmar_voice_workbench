//! `wb-engine bench`: speed and memory of one model on one clip, on one device with one thread
//! count. The speed tables of the phase are built from this (`docs/evidence/phase-2/`).
//!
//! A run is the whole pipeline the batch engine runs on a chunked clip with the language given:
//! VAD, chunking and decoding every chunk. Model loading is timed on its own. One warm-up run
//! comes first (caches, GPU pipelines, first-use allocations); the timed runs follow and the
//! median is reported. If the warm-up is slower than `single_run_above_rtf` only one timed run is
//! made and the report says so.

use std::time::Instant;

use anyhow::Result;
use wb_engine_core::chunker::{self, Params};
use wb_protocol::{BenchMode, BenchReport, BenchRequest, Device, Settings};

use crate::asr::{Model, Vad};
use crate::{audio, info, win};

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

/// The settings a mode implies, on top of the request's own.
fn mode_settings(req: &BenchRequest) -> Settings {
    let mut s = req.settings.clone();
    match req.mode {
        BenchMode::Rolling => {
            s.beam_size = 5;
            s.best_of = 5;
            s.temperature_fallback = true;
            s.max_chunk_s = 15.0;
        }
        BenchMode::NearLive => {
            s.beam_size = 1;
            s.best_of = 1;
            s.temperature_fallback = false;
            s.max_chunk_s = 15.0;
        }
    }
    s
}

pub fn run(
    req: &BenchRequest,
    gpu_ordinal: Option<u32>,
    device_name: &str,
    adapter_name: Option<&str>,
) -> Result<BenchReport> {
    let threads = req.threads.max(1) as usize;
    let s = mode_settings(req);
    let pcm = audio::read_wav_16k(std::path::Path::new(&req.clip))?;
    let clip_seconds = pcm.len() as f64 / 16_000.0;

    let load_started = Instant::now();
    let mut model = Model::load(&req.model, req.device, gpu_ordinal, s.flash_attn)?;
    let mut vad = Vad::load(&req.vad_model.path, threads.min(4))?;
    let load_s = load_started.elapsed().as_secs_f64();

    let params = Params {
        max_chunk_s: f64::from(s.max_chunk_s),
        sentence_split: false,
        ..Params::default()
    };
    let once = |model: &mut Model, vad: &mut Vad| -> Result<(f64, u32, u32)> {
        let started = Instant::now();
        let speech = vad.speech(&pcm, &s)?;
        let chunks = chunker::chunks(&speech, &pcm, 16_000, &params);
        let (mut segments, mut truncated) = (0u32, 0u32);
        let mut context = String::new();
        for c in &chunks {
            let a = (c.start_ms as usize * 16).min(pcm.len());
            let b = (c.end_ms as usize * 16).min(pcm.len());
            if b <= a + 1_600 {
                continue;
            }
            let prompt = if s.previous_text_context {
                format!("{} {}", s.glossary, context).trim().to_string()
            } else {
                s.glossary.clone()
            };
            let d = model.decode(&pcm[a..b], &req.lang, &prompt, &s, threads)?;
            if !d.text.text.trim().is_empty() {
                segments += 1;
                context = d
                    .text
                    .text
                    .chars()
                    .rev()
                    .take(s.context_chars)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
            }
            if d.truncated {
                truncated += 1;
            }
        }
        Ok((
            started.elapsed().as_secs_f64() / clip_seconds.max(0.001),
            segments,
            truncated,
        ))
    };

    let (warmup_rtf, _, _) = once(&mut model, &mut vad)?;
    let single = req.single_run_above_rtf.is_some_and(|cap| warmup_rtf > cap);
    let runs = if single { 1 } else { req.runs.max(1) };
    let mut rtfs = Vec::with_capacity(runs as usize);
    let (mut segments, mut truncated) = (0, 0);
    for _ in 0..runs {
        let (rtf, seg, trunc) = once(&mut model, &mut vad)?;
        rtfs.push(rtf);
        segments = seg;
        truncated = trunc;
    }
    let mut sorted = rtfs.clone();
    let rtf_median = median(&mut sorted);
    let (peak_ws, peak_commit) = win::peak_memory();
    let (gpu_local, gpu_shared) = match (req.device, adapter_name) {
        (Device::Gpu { .. }, Some(name)) => match win::gpu_memory_in_use(name) {
            Some((l, n)) => (Some(l), Some(n)),
            None => (None, None),
        },
        _ => (None, None),
    };
    Ok(BenchReport {
        model: req.model.id.clone(),
        variant: info::variant().to_string(),
        whisper_cpp: info::whisper_cpp(whisper_rs::get_whisper_version()),
        device: req.device,
        device_name: device_name.to_string(),
        threads: req.threads,
        mode: req.mode,
        clip_seconds,
        lang: req.lang.clone(),
        load_s,
        rtf_runs: rtfs,
        rtf_median,
        warmup_rtf,
        single_run: single,
        peak_working_set_bytes: peak_ws,
        peak_commit_bytes: peak_commit,
        gpu_local_bytes: gpu_local,
        gpu_shared_bytes: gpu_shared,
        segments,
        truncated_chunks: truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_of_odd_and_even_counts() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&mut []), 0.0);
        assert_eq!(median(&mut [7.0]), 7.0);
    }
}
