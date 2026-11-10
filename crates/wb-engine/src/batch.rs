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
use wb_protocol::{BatchRequest, DoneSummary, Event, LangProb, SegmentEvent, SegmentFlag, Stats};

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
                    loaded.push(Model::load(model, req.device, gpu_ordinal)?);
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
    };
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

            let t = attempt.decoded.text.text.trim().to_string();
            let counts = attempt.counts;
            let ratio = guards::compression_ratio(&t);
            let metrics = Metrics {
                avg_logprob: attempt.decoded.avg_logprob,
                no_speech_prob: attempt.decoded.no_speech_prob,
                compression_ratio: ratio,
                speech_ratio: c.speech_ratio() as f32,
            };
            let mut flags: Vec<SegmentFlag> = Vec::new();
            if attempt.decoded.truncated {
                flags.push(SegmentFlag::Truncated);
                summary.truncated_chunks += 1;
                out::warn(
                    "truncated_chunk",
                    format!(
                        "chunk {chunk_no} ({} to {} ms) reached the token limit; its text may be cut",
                        abs(c.start_ms),
                        abs(c.end_ms)
                    ),
                );
            }
            if attempt.decoded.text.is_broken() {
                flags.push(SegmentFlag::BrokenUtf8);
                out::warn(
                    "broken_utf8",
                    format!(
                        "chunk {chunk_no}: the model output ended inside a character; the incomplete bytes were dropped"
                    ),
                );
            }
            let dropped = guards::drop_reason(&metrics);
            if let Some(reason) = dropped {
                out::warn(
                    "segment_dropped",
                    format!(
                        "chunk {chunk_no} ({} to {} ms) dropped by a guard: {reason:?}",
                        abs(c.start_ms),
                        abs(c.end_ms)
                    ),
                );
                // A dropped chunk must not poison the next prompt.
                context.remove(&attempt.lang);
            } else if !t.is_empty() {
                flags.extend(guards::flags(&t, &attempt.lang, &counts));
                let flagged = flags.iter().any(|f| {
                    matches!(
                        f,
                        SegmentFlag::Repetition
                            | SegmentFlag::SilencePhrase
                            | SegmentFlag::WrongScript
                            | SegmentFlag::CompressionRatio
                    )
                });
                let prob = det.prob_of(&attempt.lang);
                let lang_prob = if locked.is_some() || det.probs.is_empty() || !multi {
                    1.0
                } else {
                    prob
                };
                let ev = SegmentEvent {
                    unit: unit.index,
                    chunk: chunk_no,
                    start_ms: abs(c.start_ms),
                    end_ms: abs(c.end_ms),
                    text: t.clone(),
                    lang: attempt.lang.clone(),
                    lang_prob,
                    mixed: text::is_mixed(&counts),
                    dir: text::direction(&counts).to_string(),
                    avg_logprob: metrics.avg_logprob,
                    no_speech_prob: metrics.no_speech_prob,
                    compression_ratio: ratio,
                    model: attempt.model_id.clone(),
                    decoded_both_ways: chose_both,
                    detection: det
                        .probs
                        .iter()
                        .map(|(l, p)| LangProb {
                            lang: l.clone(),
                            prob: *p,
                        })
                        .collect(),
                    flags: flags.clone(),
                    truncated: attempt.decoded.truncated,
                };
                sink.send(&Event::Segment(ev))?;
                summary.segments += 1;
                if flagged {
                    context.remove(&attempt.lang);
                } else {
                    let entry = context.entry(attempt.lang.clone()).or_default();
                    entry.push(' ');
                    entry.push_str(&t);
                    *entry = tail_chars(entry.trim(), s.context_chars);
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
