//! `wb-engine batch`: transcribes work units (or one file) and reports segments on the audio clock.
//!
//! Per unit: voice activity detection, chunks of 3 to 15 s cut at pauses (`wb-engine-core`), the
//! language of each chunk among the session's languages, decoding with that language's model
//! (both ways when two languages are close), the output guards, and same-language context for the
//! next chunk. The engine writes no transcript files: the app is the single writer.

use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::{Result, anyhow};
use wb_engine_core::chunker::{self, Chunk, Params};
use wb_engine_core::guards::{self, Metrics};
use wb_engine_core::lang::{self, Candidate, Detection, Plan, Reason, Rules};
use wb_engine_core::text::{self, ScriptCounts};
use wb_protocol::{BatchRequest, DoneSummary, Event, LangProb, SegmentEvent, SegmentFlag, Source, Stats};

use crate::asr::{Decoded, Model, Vad};
use crate::out;
use crate::source::Reader;
use crate::win;

/// Models by language, loaded once.
struct Models {
    loaded: Vec<Model>,
    /// language (or "*") -> index in `loaded`
    by_lang: BTreeMap<String, usize>,
}

impl Models {
    fn load(req: &BatchRequest, gpu_ordinal: Option<u32>) -> Result<Self> {
        let mut loaded: Vec<Model> = Vec::new();
        let mut by_lang = BTreeMap::new();
        let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
        for (lang, model) in &req.models_by_lang {
            let idx = match by_id.get(&model.id) {
                Some(i) => *i,
                None => {
                    loaded.push(Model::load(model, req.device, gpu_ordinal, req.settings.flash_attn)?);
                    by_id.insert(model.id.clone(), loaded.len() - 1);
                    loaded.len() - 1
                }
            };
            by_lang.insert(lang.clone(), idx);
        }
        Ok(Self { loaded, by_lang })
    }

    fn index_for(&self, lang: &str) -> Result<usize> {
        self.by_lang
            .get(lang)
            .or_else(|| self.by_lang.get("*"))
            .copied()
            .ok_or_else(|| anyhow!("no model for language {lang}"))
    }

    /// The model that detects languages: the one for English (a multilingual model), else the
    /// catch-all, else the first.
    fn detector_index(&self) -> usize {
        self.by_lang
            .get("en")
            .or_else(|| self.by_lang.get("*"))
            .copied()
            .unwrap_or(0)
    }

    fn load_s(&self) -> f64 {
        self.loaded.iter().map(|m| m.load_s).sum()
    }
}

/// One decode attempt of a chunk in one language.
struct Attempt {
    lang: String,
    model_id: String,
    decoded: Decoded,
    counts: ScriptCounts,
}

fn candidate(a: &Attempt) -> Candidate {
    Candidate {
        lang: a.lang.clone(),
        avg_logprob: a.decoded.avg_logprob,
        wrong_script: text::script_contradicts(&a.lang, &a.counts),
        empty: a.decoded.text.text.trim().is_empty(),
    }
}

fn prompt_for(glossary: &str, context: &str) -> String {
    match (glossary.is_empty(), context.is_empty()) {
        (true, true) => String::new(),
        (false, true) => glossary.to_string(),
        (true, false) => context.to_string(),
        (false, false) => format!("{glossary} {context}"),
    }
}

/// The last `n` characters of `s` (characters, not bytes: Devanagari and Arabic are multi-byte).
fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// Why a decode of a chunk (or half a chunk) is not usable as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// The text looks like a loop (`guards::compression_ratio` over the threshold).
    Loop,
    /// The decoder reached its token limit: the text is cut.
    TokenLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Good,
    /// Not speech (little speech in the chunk, or the model says so): dropped with a warning event, as the
    /// PLAN says. Not retried: nothing is lost.
    Skip(guards::DropReason),
    /// Speech that the decoder failed on: goes through the retry ladder.
    Fail(Failure),
}

fn judge(a: &Attempt, speech_ratio: f32) -> (Verdict, Metrics) {
    let t = a.decoded.text.text.trim();
    let metrics = Metrics {
        avg_logprob: a.decoded.avg_logprob,
        no_speech_prob: a.decoded.no_speech_prob,
        compression_ratio: guards::compression_ratio(t),
        speech_ratio,
    };
    let verdict = match guards::drop_reason(&metrics) {
        Some(guards::DropReason::Compression) => Verdict::Fail(Failure::Loop),
        Some(reason) => Verdict::Skip(reason),
        None if a.decoded.truncated => Verdict::Fail(Failure::TokenLimit),
        None => Verdict::Good,
    };
    (verdict, metrics)
}

/// Development aid for calibrating the loop threshold: with `WB_ATTEMPT_LOG=FILE` every decode attempt
/// is appended to FILE as one JSON line, including its TEXT. Never set in the app; the file is scratch.
struct AttemptLog {
    path: Option<String>,
    source: String,
}

impl AttemptLog {
    fn new(source: &Source) -> Self {
        let name = match source {
            Source::File { path } => std::path::Path::new(path)
                .file_stem()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned()),
            Source::Session { dir } => dir.clone(),
        };
        Self {
            path: std::env::var("WB_ATTEMPT_LOG").ok().filter(|p| !p.is_empty()),
            source: name,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record(&self, chunk: u32, start_ms: u64, end_ms: u64, step: &str, a: &Attempt, m: &Metrics, v: &Verdict) {
        use std::io::Write;
        let Some(path) = &self.path else { return };
        let line = serde_json::json!({
            "source": self.source, "chunk": chunk, "start_ms": start_ms, "end_ms": end_ms, "step": step,
            "lang": a.lang, "text": a.decoded.text.text.trim(), "chars": a.decoded.text.text.trim().chars().count(),
            "ratio": m.compression_ratio, "ratio_utf8": guards::compression_ratio_utf8(a.decoded.text.text.trim()),
            "avg_logprob": m.avg_logprob, "no_speech_prob": m.no_speech_prob, "speech_ratio": m.speech_ratio,
            "truncated": a.decoded.truncated, "verdict": format!("{v:?}"),
        });
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// The result for one half of a chunk in retry step (b).
enum Half {
    Text(Box<Attempt>, Metrics),
    Failed(Failure),
    /// Not speech (under 30% speech, or the model says so): nothing to emit.
    Silent,
}

/// What every segment of one chunk shares.
struct ChunkCtx<'a> {
    unit: u32,
    chunk_no: u32,
    det: &'a Detection,
    locked: bool,
    multi: bool,
    chose_both: bool,
    context_chars: usize,
}

impl ChunkCtx<'_> {
    fn lang_prob(&self, lang: &str) -> f32 {
        if self.locked || self.det.probs.is_empty() || !self.multi {
            1.0
        } else {
            self.det.prob_of(lang)
        }
    }

    fn detection(&self) -> Vec<LangProb> {
        self.det
            .probs
            .iter()
            .map(|(l, p)| LangProb { lang: l.clone(), prob: *p })
            .collect()
    }
}

/// Emits the text of a decode that passed the guards (no event for an empty text).
#[allow(clippy::too_many_arguments)]
fn emit_text(
    sink: &mut Sink<'_>,
    summary: &mut DoneSummary,
    context: &mut BTreeMap<String, String>,
    cx: &ChunkCtx<'_>,
    a: &Attempt,
    m: &Metrics,
    start_ms: u64,
    end_ms: u64,
    retried: bool,
) -> Result<()> {
    let t = a.decoded.text.text.trim().to_string();
    if t.is_empty() {
        return Ok(());
    }
    let mut flags: Vec<SegmentFlag> = Vec::new();
    if a.decoded.text.is_broken() {
        flags.push(SegmentFlag::BrokenUtf8);
        out::warn(
            "broken_utf8",
            format!(
                "chunk {}: the model output ended inside a character; the incomplete bytes were dropped",
                cx.chunk_no
            ),
        );
    }
    if retried {
        flags.push(SegmentFlag::Retried);
    }
    flags.extend(guards::flags(&t, &a.lang, &a.counts));
    let flagged = flags.iter().any(|f| {
        matches!(
            f,
            SegmentFlag::Repetition
                | SegmentFlag::SilencePhrase
                | SegmentFlag::WrongScript
                | SegmentFlag::CompressionRatio
        )
    });
    sink.send(&Event::Segment(SegmentEvent {
        unit: cx.unit,
        chunk: cx.chunk_no,
        start_ms,
        end_ms,
        text: t.clone(),
        lang: a.lang.clone(),
        lang_prob: cx.lang_prob(&a.lang),
        mixed: text::is_mixed(&a.counts),
        dir: text::direction(&a.counts).to_string(),
        avg_logprob: m.avg_logprob,
        no_speech_prob: m.no_speech_prob,
        compression_ratio: m.compression_ratio,
        model: a.model_id.clone(),
        decoded_both_ways: cx.chose_both,
        detection: cx.detection(),
        flags,
        truncated: a.decoded.truncated,
    }))?;
    summary.segments += 1;
    if retried {
        summary.retried_chunks += 1;
    }
    if flagged {
        context.remove(&a.lang);
    } else {
        let entry = context.entry(a.lang.clone()).or_default();
        entry.push(' ');
        entry.push_str(&t);
        *entry = tail_chars(entry.trim(), cx.context_chars);
    }
    Ok(())
}

/// A stretch that failed the whole retry ladder: a segment with its times, the flag `unclear` and no
/// text, so review shows the gap. Never a silent loss (PLAN section 6).
#[allow(clippy::too_many_arguments)]
fn emit_unclear(
    sink: &mut Sink<'_>,
    summary: &mut DoneSummary,
    context: &mut BTreeMap<String, String>,
    cx: &ChunkCtx<'_>,
    lang: &str,
    model_id: &str,
    failure: Failure,
    start_ms: u64,
    end_ms: u64,
) -> Result<()> {
    out::warn(
        "chunk_unclear",
        format!(
            "chunk {} ({start_ms} to {end_ms} ms) could not be decoded cleanly after the retries ({failure:?}); emitted as unclear with no text",
            cx.chunk_no
        ),
    );
    sink.send(&Event::Segment(SegmentEvent {
        unit: cx.unit,
        chunk: cx.chunk_no,
        start_ms,
        end_ms,
        text: String::new(),
        lang: lang.to_string(),
        lang_prob: cx.lang_prob(lang),
        mixed: false,
        dir: "auto".to_string(),
        avg_logprob: 0.0,
        no_speech_prob: 0.0,
        compression_ratio: 0.0,
        model: model_id.to_string(),
        decoded_both_ways: false,
        detection: cx.detection(),
        flags: vec![
            SegmentFlag::Unclear,
            match failure {
                Failure::Loop => SegmentFlag::CompressionRatio,
                Failure::TokenLimit => SegmentFlag::Truncated,
            },
        ],
        truncated: failure == Failure::TokenLimit,
    }))?;
    summary.segments += 1;
    summary.unclear_chunks += 1;
    if failure == Failure::TokenLimit {
        summary.truncated_chunks += 1;
    }
    context.remove(lang);
    Ok(())
}

pub struct Sink<'a> {
    pub emit: &'a mut dyn FnMut(&Event) -> std::io::Result<()>,
}

impl Sink<'_> {
    pub fn send(&mut self, e: &Event) -> Result<()> {
        (self.emit)(e).map_err(|err| anyhow!("the app closed the event stream: {err}"))
    }

    /// A fatal `error` event (best effort: the stream may be gone).
    pub fn send_error(&mut self, code: &str, message: &str) -> Result<()> {
        self.send(&Event::Error {
            code: code.into(),
            message: message.into(),
            fatal: true,
        })
    }
}

/// Runs a batch request to the end. Returns the summary.
pub fn run(
    req: &BatchRequest,
    gpu_ordinal: Option<u32>,
    device_name: &str,
    sink: &mut Sink<'_>,
) -> Result<DoneSummary> {
    let started = Instant::now();
    let threads = req.threads.max(1) as usize;
    let reader = Reader::open(&req.source)?;
    let mut models = Models::load(req, gpu_ordinal)?;
    let mut vad = Vad::load(&req.vad_model.path, threads.min(4))?;
    sink.send(&Event::Ready {
        device: req.device,
        device_name: device_name.to_string(),
        threads: req.threads,
        models: models.loaded.iter().map(|m| m.id.clone()).collect(),
        load_s: models.load_s(),
    })?;

    let s = &req.settings;
    let multi = req.languages.len() > 1 && req.detect_language;
    let params = Params {
        max_chunk_s: f64::from(s.max_chunk_s),
        sentence_split: s.sentence_split && multi,
        ..Params::default()
    };
    let rules = Rules::default();
    let mut summary = DoneSummary {
        segments: 0,
        chunks: 0,
        audio_ms: 0,
        truncated_chunks: 0,
        elapsed_s: 0.0,
        retried_chunks: 0,
        unclear_chunks: 0,
    };
    let attempt_log = AttemptLog::new(&req.source);
    // Previous text per language, for the next chunk's prompt; reset after a flagged segment.
    let mut context: BTreeMap<String, String> = BTreeMap::new();
    let mut prev_lang: Option<String> = None;
    let mut chunk_no = 0u32;

    for unit in reader.units(&req.units) {
        let unit_started = Instant::now();
        let audio = reader.unit_audio(unit.start_ms, unit.end_ms)?;
        if audio.pcm.is_empty() {
            continue;
        }
        let speech = vad.speech(&audio.pcm, s)?;
        let chunks: Vec<Chunk> = chunker::chunks(&speech, &audio.pcm, 16_000, &params);
        let abs = |ms: u64| audio.start_ms + ms;
        let slice = |c: &Chunk| -> &[f32] {
            let a = (c.start_ms as usize * 16).min(audio.pcm.len());
            let b = (c.end_ms as usize * 16).min(audio.pcm.len());
            &audio.pcm[a..b.max(a)]
        };
        let todo: Vec<&Chunk> = chunks
            .iter()
            .filter(|c| abs(c.end_ms) > req.resume_from_ms)
            .collect();

        // First pass: language detection for every chunk of the unit, so each decision can see the
        // neighbours' languages (rule: a short unsure chunk takes its neighbours').
        let mut detections: Vec<Option<Detection>> = Vec::with_capacity(todo.len());
        let mut detect_threads = threads;
        if detect_threads == 0 {
            detect_threads = 1;
        }
        for c in &todo {
            let locked = lang::locked_language(&req.lang_log, abs(c.start_ms));
            if !multi || locked.is_some() || c.duration_s() < 0.3 {
                detections.push(None);
                continue;
            }
            let idx = models.detector_index();
            let raw = models.loaded[idx].detect_languages(slice(c), detect_threads)?;
            detections.push(Some(Detection::restrict(&raw, &req.languages)));
        }

        // Second pass: decide, decode, guard, emit.
        let mut detections_done = 0u32;
        let mut both_ways = 0u32;
        let mut decode_s = 0.0f64;
        for (i, c) in todo.iter().enumerate() {
            let chunk_pcm = slice(c);
            if chunk_pcm.len() < 1_600 {
                continue; // under 100 ms: whisper.cpp refuses it
            }
            let locked = lang::locked_language(&req.lang_log, abs(c.start_ms)).map(str::to_string);
            let det = detections[i]
                .clone()
                .unwrap_or_else(|| Detection::restrict(&[], &req.languages));
            if detections[i].is_some() {
                detections_done += 1;
            }
            let next_guess = detections
                .get(i + 1)
                .and_then(|d| d.as_ref())
                .and_then(|d| d.best().map(|b| b.0.clone()));
            let plan = lang::decide(
                &det,
                c.duration_s() as f32,
                locked.as_deref(),
                prev_lang.as_deref(),
                next_guess.as_deref(),
                &req.languages,
                &rules,
            );
            let t0 = Instant::now();
            let (attempt, chose_both, lang_reason) = match &plan {
                Plan::Single { lang, reason } => (
                    decode(&mut models, lang, chunk_pcm, &context, req, threads)?,
                    false,
                    Some(*reason),
                ),
                Plan::Both { first, second } => {
                    both_ways += 1;
                    let a = decode(&mut models, first, chunk_pcm, &context, req, threads)?;
                    let b = decode(&mut models, second, chunk_pcm, &context, req, threads)?;
                    let (ca, cb) = (candidate(&a), candidate(&b));
                    let pick_b = std::ptr::eq(lang::pick_better(&ca, &cb, &det, &rules), &cb);
                    (if pick_b { b } else { a }, true, None)
                }
            };
            decode_s += t0.elapsed().as_secs_f64();
            let mut attempt = attempt;
            // Output in the wrong script is decoded once more without the prompt (a bad prompt can
            // pull the model into the wrong script); what remains is flagged.
            if text::script_contradicts(&attempt.lang, &attempt.counts)
                && !attempt.decoded.text.text.trim().is_empty()
            {
                let mut retry_req = req.clone();
                retry_req.settings.glossary = String::new();
                let retry = decode(
                    &mut models,
                    &attempt.lang.clone(),
                    chunk_pcm,
                    &BTreeMap::new(),
                    &retry_req,
                    threads,
                )?;
                if !text::script_contradicts(&retry.lang, &retry.counts) {
                    attempt = retry;
                }
            }

            let speech_ratio = c.speech_ratio() as f32;
            let (start_abs, end_abs) = (abs(c.start_ms), abs(c.end_ms));
            let (verdict, metrics) = judge(&attempt, speech_ratio);
            attempt_log.record(chunk_no, start_abs, end_abs, "first", &attempt, &metrics, &verdict);
            let cx = ChunkCtx {
                unit: unit.index,
                chunk_no,
                det: &det,
                locked: locked.is_some(),
                multi,
                chose_both,
                context_chars: s.context_chars,
            };
            match verdict {
                Verdict::Good => {
                    emit_text(sink, &mut summary, &mut context, &cx, &attempt, &metrics, start_abs, end_abs, false)?;
                }
                Verdict::Skip(reason) => {
                    out::warn(
                        "segment_dropped",
                        format!(
                            "chunk {chunk_no} ({start_abs} to {end_abs} ms) is not speech and was dropped: {reason:?} (compression ratio {:.2}, average log-probability {:.2}, no-speech {:.2}, speech share {:.2})",
                            metrics.compression_ratio,
                            metrics.avg_logprob,
                            metrics.no_speech_prob,
                            metrics.speech_ratio
                        ),
                    );
                    // A dropped chunk must not poison the next prompt.
                    context.remove(&attempt.lang);
                }
                Verdict::Fail(failure) => {
                    // The retry ladder (PLAN section 6): (a) again without previous-text context, with
                    // temperature fallback; (b) each half, cut at the deepest VAD pause, without context;
                    // (c) only then a segment flagged `unclear`, with no text. Nothing is dropped silently.
                    out::warn(
                        "chunk_retry",
                        format!(
                            "chunk {chunk_no} ({start_abs} to {end_abs} ms) failed a guard ({failure:?}; compression ratio {:.2}, {} characters); retrying without previous-text context",
                            metrics.compression_ratio,
                            attempt.decoded.text.text.trim().chars().count()
                        ),
                    );
                    let lang = attempt.lang.clone();
                    let mut retry_req = req.clone();
                    retry_req.settings.temperature_fallback = true;
                    let no_context = BTreeMap::new();
                    let step_a = decode(&mut models, &lang, chunk_pcm, &no_context, &retry_req, threads)?;
                    let (va, ma) = judge(&step_a, speech_ratio);
                    attempt_log.record(chunk_no, start_abs, end_abs, "a_no_context", &step_a, &ma, &va);
                    if va == Verdict::Good {
                        out::warn(
                            "chunk_rescued",
                            format!("chunk {chunk_no}: rescued by step a (decoded again without previous-text context)"),
                        );
                        emit_text(sink, &mut summary, &mut context, &cx, &step_a, &ma, start_abs, end_abs, true)?;
                    } else if let Some(split_ms) = chunker::split_point_ms(&speech, c, &audio.pcm, 16_000) {
                        let cut = ((split_ms.saturating_sub(c.start_ms)) as usize * 16).min(chunk_pcm.len());
                        let halves = [(c.start_ms, split_ms, &chunk_pcm[..cut]), (split_ms, c.end_ms, &chunk_pcm[cut..])];
                        let mut outcome: Vec<(u64, u64, Half)> = Vec::new();
                        for (n, (a_ms, b_ms, pcm)) in halves.into_iter().enumerate() {
                            let dur_s = (b_ms - a_ms) as f64 / 1000.0;
                            let ratio = (chunker::speech_seconds(&speech, a_ms as f64 / 1000.0, b_ms as f64 / 1000.0)
                                / dur_s.max(1e-9)) as f32;
                            let half = if pcm.len() < 1_600 || ratio < guards::MIN_SPEECH_RATIO {
                                Half::Silent
                            } else {
                                let d = decode(&mut models, &lang, pcm, &no_context, &retry_req, threads)?;
                                let (vh, mh) = judge(&d, ratio);
                                attempt_log.record(
                                    chunk_no,
                                    abs(a_ms),
                                    abs(b_ms),
                                    if n == 0 { "b_half1" } else { "b_half2" },
                                    &d,
                                    &mh,
                                    &vh,
                                );
                                match vh {
                                    Verdict::Good => Half::Text(Box::new(d), mh),
                                    Verdict::Skip(_) => Half::Silent,
                                    Verdict::Fail(f) => Half::Failed(f),
                                }
                            };
                            outcome.push((a_ms, b_ms, half));
                        }
                        if outcome.iter().all(|(_, _, h)| matches!(h, Half::Failed(_))) {
                            emit_unclear(sink, &mut summary, &mut context, &cx, &lang, &step_a.model_id, failure, start_abs, end_abs)?;
                        } else {
                            out::warn(
                                "chunk_rescued",
                                format!("chunk {chunk_no}: rescued by step b (cut at {} ms, each half decoded without context)", abs(split_ms)),
                            );
                            for (a_ms, b_ms, half) in outcome {
                                match half {
                                    Half::Text(d, m) => {
                                        emit_text(sink, &mut summary, &mut context, &cx, &d, &m, abs(a_ms), abs(b_ms), true)?;
                                    }
                                    Half::Failed(f) => {
                                        emit_unclear(sink, &mut summary, &mut context, &cx, &lang, &step_a.model_id, f, abs(a_ms), abs(b_ms))?;
                                    }
                                    Half::Silent => {}
                                }
                            }
                        }
                    } else {
                        emit_unclear(sink, &mut summary, &mut context, &cx, &lang, &step_a.model_id, failure, start_abs, end_abs)?;
                    }
                }
            }
            let _ = lang_reason;
            if !matches!(lang_reason, Some(Reason::Locked)) || locked.is_none() {
                prev_lang = Some(attempt.lang.clone());
            } else {
                prev_lang = locked.clone();
            }
            summary.chunks += 1;
            chunk_no += 1;
            sink.send(&Event::Progress {
                unit: unit.index,
                committed_until_ms: abs(c.end_ms),
                unit_end_ms: unit.end_ms,
            })?;
        }
        let audio_ms = audio.duration_ms();
        summary.audio_ms += audio_ms;
        let (peak_ws, _peak_commit) = win::peak_memory();
        sink.send(&Event::Stats(Stats {
            unit: unit.index,
            chunks: todo.len() as u32,
            audio_ms,
            decode_s: unit_started.elapsed().as_secs_f64(),
            rtf: if audio_ms == 0 {
                0.0
            } else {
                unit_started.elapsed().as_secs_f64() / (audio_ms as f64 / 1000.0)
            },
            peak_working_set_bytes: peak_ws,
            detections: detections_done,
            both_ways,
        }))?;
        let _ = decode_s;
        sink.send(&Event::Progress {
            unit: unit.index,
            committed_until_ms: unit.end_ms,
            unit_end_ms: unit.end_ms,
        })?;
    }
    summary.elapsed_s = started.elapsed().as_secs_f64();
    sink.send(&Event::Done(summary.clone()))?;
    Ok(summary)
}

fn decode(
    models: &mut Models,
    lang: &str,
    pcm: &[f32],
    context: &BTreeMap<String, String>,
    req: &BatchRequest,
    threads: usize,
) -> Result<Attempt> {
    let idx = models.index_for(lang)?;
    let s = &req.settings;
    let ctx = if s.previous_text_context {
        context.get(lang).map_or("", String::as_str)
    } else {
        ""
    };
    let prompt = prompt_for(&s.glossary, ctx);
    let decoded = models.loaded[idx].decode(pcm, lang, &prompt, s, threads)?;
    let counts = text::script_counts(&decoded.text.text);
    Ok(Attempt {
        lang: lang.to_string(),
        model_id: models.loaded[idx].id.clone(),
        decoded,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_keeps_the_last_characters_not_bytes() {
        let hindi = "नमस्ते दोस्तों आज हम बात करेंगे";
        let t = tail_chars(hindi, 10);
        assert_eq!(t.chars().count(), 10);
        assert!(hindi.ends_with(&t));
        assert_eq!(tail_chars("short", 200), "short");
        assert_eq!(tail_chars("", 5), "");
    }

    #[test]
    fn the_prompt_is_the_glossary_then_the_same_language_context() {
        assert_eq!(prompt_for("", ""), "");
        assert_eq!(prompt_for("STELMAR", ""), "STELMAR");
        assert_eq!(prompt_for("", "previous text"), "previous text");
        assert_eq!(
            prompt_for("STELMAR", "previous text"),
            "STELMAR previous text"
        );
    }
}
