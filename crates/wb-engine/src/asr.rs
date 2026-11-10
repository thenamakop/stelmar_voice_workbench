//! whisper.cpp through whisper-rs: loading models, voice activity detection, language detection and
//! decoding one chunk. Safe code only; the unsafe parts of whisper-rs are inside that crate.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use wb_engine_core::chunker::Speech;
use wb_engine_core::text::{Utf8Text, join_utf8};
use wb_protocol::{Device, ModelRef, Settings};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    WhisperVadContext, WhisperVadContextParams, WhisperVadParams, get_lang_max_id, get_lang_str,
};

/// The model emits its first special token (end of text) at this id in every multilingual Whisper
/// vocabulary; tokens from here on are not text.
const FIRST_SPECIAL_TOKEN: i32 = 50_257;
/// Whisper stops a decode at half its text context (224 tokens); a chunk that reaches about this
/// many has probably been cut (`docs/GATES.md`, 5a -> 2).
pub const TOKEN_LIMIT: u32 = 220;

/// A model or the VAD model could not be loaded (exit code 4 for the app).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LoadError(pub String);

/// One loaded model with its decoding state.
pub struct Model {
    pub id: String,
    pub load_s: f64,
    state: WhisperState,
    // The state holds the context alive; kept for the model's name in logs.
    _ctx: WhisperContext,
}

impl Model {
    /// Loads a model on the CPU or on GPU number `gpu_ordinal` of ggml's list (see `devices.rs`).
    pub fn load(model: &ModelRef, device: Device, gpu_ordinal: Option<u32>) -> Result<Self> {
        let started = Instant::now();
        let mut params = WhisperContextParameters::default();
        match device {
            Device::Cpu => {
                params.use_gpu(false);
            }
            Device::Gpu { .. } => {
                let ordinal =
                    gpu_ordinal.ok_or_else(|| anyhow!("no GPU ordinal for {device:?}"))?;
                params.use_gpu(true);
                params.gpu_device(i32::try_from(ordinal).unwrap_or(0));
            }
        }
        let path = Path::new(&model.path);
        let ctx = WhisperContext::new_with_params(&model.path, params).map_err(|e| {
            LoadError(format!(
                "could not load model {} from {}: {e}",
                model.id,
                path.display()
            ))
        })?;
        let state = ctx.create_state().map_err(|e| {
            LoadError(format!(
                "could not create the decoding state for {}: {e}",
                model.id
            ))
        })?;
        Ok(Self {
            id: model.id.clone(),
            load_s: started.elapsed().as_secs_f64(),
            state,
            _ctx: ctx,
        })
    }

    /// Whisper's probability for every language it knows, for `pcm` (16 kHz mono).
    pub fn detect_languages(&mut self, pcm: &[f32], threads: usize) -> Result<Vec<(String, f32)>> {
        self.state
            .pcm_to_mel(pcm, threads)
            .map_err(|e| anyhow!("mel spectrogram failed: {e}"))?;
        let (_best, probs) = self
            .state
            .lang_detect(0, threads)
            .map_err(|e| anyhow!("language detection failed: {e}"))?;
        let max = get_lang_max_id();
        Ok((0..=max)
            .filter_map(|id| {
                let code = get_lang_str(id)?;
                let p = *probs.get(usize::try_from(id).ok()?)?;
                Some((code.to_string(), p))
            })
            .collect())
    }

    /// Decodes `pcm` (16 kHz mono) as `lang` with the given prompt (glossary and same-language
    /// context, empty for none).
    pub fn decode(
        &mut self,
        pcm: &[f32],
        lang: &str,
        prompt: &str,
        s: &Settings,
        threads: usize,
    ) -> Result<Decoded> {
        let strategy = if s.beam_size > 1 {
            SamplingStrategy::BeamSearch {
                beam_size: s.beam_size,
                patience: -1.0,
            }
        } else {
            SamplingStrategy::Greedy {
                best_of: s.best_of.max(1),
            }
        };
        let mut p = FullParams::new(strategy);
        p.set_n_threads(i32::try_from(threads).unwrap_or(1));
        p.set_language(Some(lang));
        p.set_translate(false);
        p.set_no_timestamps(!s.timestamps);
        p.set_single_segment(false);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        p.set_temperature(0.0);
        p.set_temperature_inc(if s.temperature_fallback { 0.2 } else { 0.0 });
        // Past text as a prompt is the app's own context (same language, last characters), passed in
        // `prompt`; without any prompt the model is told to use none (n_max_text_ctx = 0).
        p.set_no_context(true);
        if prompt.is_empty() {
            p.set_n_max_text_ctx(0);
        } else {
            p.set_initial_prompt(prompt);
        }
        self.state
            .full(p, pcm)
            .map_err(|e| anyhow!("decoding failed: {e}"))?;
        self.collect()
    }

    fn collect(&self) -> Result<Decoded> {
        let n = self.state.full_n_segments();
        let mut raw: Vec<&[u8]> = Vec::new();
        let mut logprob_sum = 0.0f64;
        let mut text_tokens = 0u32;
        let mut tokens = 0u32;
        let mut no_speech = 0.0f32;
        for i in 0..n {
            let seg = self
                .state
                .get_segment(i)
                .with_context(|| format!("segment {i} missing"))?;
            raw.push(
                seg.to_bytes()
                    .map_err(|e| anyhow!("segment text unreadable: {e}"))?,
            );
            no_speech = no_speech.max(seg.no_speech_probability());
            let count = seg.n_tokens();
            tokens += u32::try_from(count).unwrap_or(0);
            for t in 0..count {
                if let Some(tok) = seg.get_token(t)
                    && tok.token_id() < FIRST_SPECIAL_TOKEN
                {
                    logprob_sum += f64::from(tok.token_data().plog);
                    text_tokens += 1;
                }
            }
        }
        // The raw bytes of every segment are joined before they are decoded as UTF-8, so a
        // character is never cut between two segments.
        let text = join_utf8(&raw);
        let avg = if text_tokens == 0 {
            0.0
        } else {
            (logprob_sum / f64::from(text_tokens)) as f32
        };
        Ok(Decoded {
            text,
            avg_logprob: avg,
            no_speech_prob: no_speech,
            truncated: tokens >= TOKEN_LIMIT,
        })
    }
}

/// What one decode of one chunk gave.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub text: Utf8Text,
    pub avg_logprob: f32,
    pub no_speech_prob: f32,
    /// The chunk reached the model's token limit, so its text may be cut.
    pub truncated: bool,
}

/// Silero VAD (v6.2.0 through whisper.cpp).
pub struct Vad {
    ctx: WhisperVadContext,
}

impl Vad {
    pub fn load(path: &str, threads: usize) -> Result<Self> {
        let mut p = WhisperVadContextParams::new();
        p.set_n_threads(i32::try_from(threads).unwrap_or(1));
        p.set_use_gpu(false);
        let ctx = WhisperVadContext::new(path, p)
            .map_err(|e| LoadError(format!("could not load the VAD model {path}: {e}")))?;
        Ok(Self { ctx })
    }

    /// Speech stretches of `pcm` (16 kHz mono), in seconds. The model reports its own time unit;
    /// it is checked against the audio length (centiseconds in whisper.cpp's documentation) so a
    /// change in a later version cannot scale every chunk silently.
    pub fn speech(&mut self, pcm: &[f32], s: &Settings) -> Result<Vec<Speech>> {
        let mut vp = WhisperVadParams::new();
        vp.set_threshold(s.vad_threshold);
        vp.set_min_silence_duration(i32::try_from(s.vad_min_silence_ms).unwrap_or(100));
        vp.set_max_speech_duration(s.max_chunk_s);
        let segments = self
            .ctx
            .segments_from_samples(vp, pcm)
            .map_err(|e| anyhow!("voice activity detection failed: {e}"))?;
        let raw: Vec<(f64, f64)> = segments
            .map(|seg| (f64::from(seg.start), f64::from(seg.end)))
            .collect();
        let duration = pcm.len() as f64 / 16_000.0;
        let last = raw.iter().map(|r| r.1).fold(0.0, f64::max);
        let scale = if raw.is_empty() || last <= duration * 1.05 + 0.5 {
            1.0 // seconds
        } else if last <= duration * 105.0 + 50.0 {
            0.01 // centiseconds
        } else {
            return Err(anyhow!(
                "VAD segment end {last} does not fit a {duration:.1} s clip in seconds or centiseconds"
            ));
        };
        Ok(raw
            .into_iter()
            .filter(|(a, b)| b > a)
            .map(|(a, b)| Speech {
                start_s: a * scale,
                end_s: (b * scale).min(duration),
            })
            .collect())
    }
}
