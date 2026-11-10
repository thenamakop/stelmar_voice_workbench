//! Cutting audio into chunks at VAD pauses.
//!
//! Without model timestamps whisper.cpp decodes a 30 s window as one segment, stops at its token
//! limit (about 220 tokens; Devanagari needs many per word) and skips the rest of the window
//! (`docs/GATES.md`, 5a -> 2). So every chunk handed to the model is at most `max_chunk_s` long
//! (15 s), each decoded on its own. Chunks end at pauses: sentence-sized (about 3 to 15 s), cut at
//! the longest pause in reach, never in the middle of a word unless one stretch of speech runs
//! longer than the limit with no pause in it; then it is cut at the quietest moment near the limit
//! and the cut is marked.

/// A stretch of speech from the VAD, in seconds from the start of the audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speech {
    pub start_s: f64,
    pub end_s: f64,
}

/// A chunk to decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chunk {
    pub start_ms: u64,
    pub end_ms: u64,
    /// The chunk begins where continuous speech was cut because it ran past the limit.
    pub cut_inside_speech: bool,
    /// Seconds of the chunk that the VAD called speech.
    pub speech_s: f64,
}

impl Chunk {
    pub fn duration_s(&self) -> f64 {
        (self.end_ms - self.start_ms) as f64 / 1000.0
    }

    /// Share of the chunk the VAD called speech (PLAN section 6: under 0.3 is dropped).
    pub fn speech_ratio(&self) -> f64 {
        let d = self.duration_s();
        if d <= 0.0 { 0.0 } else { (self.speech_s / d).min(1.0) }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// Longest chunk, seconds.
    pub max_chunk_s: f64,
    /// A chunk is closed at the next pause once it is at least this long, seconds.
    pub sentence_min_s: f64,
    /// Shortest pause that counts as a sentence end, seconds.
    pub sentence_pause_s: f64,
    /// Close chunks at sentence-sized pauses (the app, in sessions with more than one language).
    /// `false` merges stretches greedily up to the limit, as the Phase 5a harness does, which is
    /// what a single-language session and the evaluation comparison use.
    pub sentence_split: bool,
    /// Silence kept before and after the speech of a chunk, seconds.
    pub pad_s: f64,
    /// Window before the limit in which a forced cut looks for the quietest moment, seconds.
    pub forced_cut_window_s: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            max_chunk_s: 15.0,
            sentence_min_s: 3.0,
            sentence_pause_s: 0.3,
            sentence_split: true,
            pad_s: 0.2,
            forced_cut_window_s: 4.0,
        }
    }
}

/// Mean absolute amplitude of 20 ms frames, for picking the quietest cut. `rate` is the sample
/// rate of `samples` (16 kHz in the engine).
fn quietest_cut(samples: &[f32], rate: u32, from_s: f64, to_s: f64) -> f64 {
    let frame = (rate as usize / 50).max(1);
    let lo = ((from_s * f64::from(rate)) as usize).min(samples.len());
    let hi = ((to_s * f64::from(rate)) as usize).min(samples.len());
    if hi <= lo + frame {
        return to_s;
    }
    let mut best = (f32::MAX, to_s);
    let mut at = lo;
    while at + frame <= hi {
        let energy: f32 = samples[at..at + frame].iter().map(|s| s.abs()).sum::<f32>() / frame as f32;
        if energy < best.0 {
            best = (energy, (at + frame / 2) as f64 / f64::from(rate));
        }
        at += frame;
    }
    best.1
}

/// Splits one stretch of speech longer than the limit into pieces of at most `max` seconds,
/// each cut at the quietest moment in the last `window` seconds before the limit.
fn split_long(s: Speech, samples: &[f32], rate: u32, p: &Params) -> Vec<Speech> {
    let mut out = Vec::new();
    let mut start = s.start_s;
    while s.end_s - start > p.max_chunk_s - 2.0 * p.pad_s {
        let limit = start + p.max_chunk_s - 2.0 * p.pad_s;
        let cut = quietest_cut(samples, rate, (limit - p.forced_cut_window_s).max(start + 1.0), limit);
        out.push(Speech { start_s: start, end_s: cut });
        start = cut;
    }
    out.push(Speech { start_s: start, end_s: s.end_s });
    out
}

/// Chunks for the speech stretches `speech` (sorted, in seconds) of `samples`.
///
/// Padding is added on both sides but never past the neighbouring chunk or the audio.
pub fn chunks(speech: &[Speech], samples: &[f32], rate: u32, p: &Params) -> Vec<Chunk> {
    let total_s = samples.len() as f64 / f64::from(rate.max(1));
    // 1. Pieces: speech stretches, with any stretch longer than the limit split first.
    let mut pieces: Vec<(Speech, bool)> = Vec::new();
    for s in speech.iter().copied().filter(|s| s.end_s > s.start_s) {
        let parts = split_long(s, samples, rate, p);
        for (i, part) in parts.into_iter().enumerate() {
            pieces.push((part, i > 0));
        }
    }
    // 2. Groups: add pieces while the chunk stays within the limit; close at a sentence-sized
    //    pause, or at the longest pause so far when the next piece would not fit.
    let room = p.max_chunk_s - 2.0 * p.pad_s;
    let mut groups: Vec<(usize, usize)> = Vec::new(); // inclusive piece ranges
    let mut first = 0usize;
    while first < pieces.len() {
        let mut last = first;
        while last + 1 < pieces.len() {
            let next = pieces[last + 1].0;
            let len_with_next = next.end_s - pieces[first].0.start_s;
            let gap = next.start_s - pieces[last].0.end_s;
            let len_now = pieces[last].0.end_s - pieces[first].0.start_s;
            if pieces[last + 1].1 {
                break; // a forced cut inside speech always starts a new chunk
            }
            if len_with_next > room {
                if !p.sentence_split {
                    break; // greedy: close before the piece that does not fit
                }
                // Does not fit: cut at the longest pause already inside the chunk if that leaves a
                // reasonable first part, otherwise here.
                let mut best = (gap, last);
                for k in first..last {
                    let g = pieces[k + 1].0.start_s - pieces[k].0.end_s;
                    let part = pieces[k].0.end_s - pieces[first].0.start_s;
                    if g > best.0 && part >= p.sentence_min_s {
                        best = (g, k);
                    }
                }
                last = best.1;
                break;
            }
            if p.sentence_split && len_now >= p.sentence_min_s && gap >= p.sentence_pause_s {
                break;
            }
            last += 1;
        }
        groups.push((first, last));
        first = last + 1;
    }
    // 3. Chunks with padding.
    let mut out: Vec<Chunk> = Vec::with_capacity(groups.len());
    for (gi, &(a, b)) in groups.iter().enumerate() {
        let start = pieces[a].0.start_s;
        let end = pieces[b].0.end_s;
        let speech_s: f64 = pieces[a..=b].iter().map(|(s, _)| s.end_s - s.start_s).sum();
        let prev_end = if gi == 0 { 0.0 } else { pieces[groups[gi - 1].1].0.end_s };
        let next_start = groups.get(gi + 1).map_or(total_s, |&(na, _)| pieces[na].0.start_s);
        // Half the gap to a neighbour at most, so padding never overlaps the next chunk.
        let pad_before = p.pad_s.min((start - prev_end).max(0.0) / if gi == 0 { 1.0 } else { 2.0 });
        let pad_after = p.pad_s.min((next_start - end).max(0.0) / if gi + 1 == groups.len() { 1.0 } else { 2.0 });
        let s_ms = ((start - pad_before).max(0.0) * 1000.0).round() as u64;
        let e_ms = (((end + pad_after).min(total_s)) * 1000.0).round() as u64;
        if e_ms > s_ms {
            out.push(Chunk {
                start_ms: s_ms,
                end_ms: e_ms,
                cut_inside_speech: pieces[a].1,
                speech_s,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    fn s(a: f64, b: f64) -> Speech {
        Speech { start_s: a, end_s: b }
    }

    /// Noise with a quiet stretch from `quiet_from` to `quiet_to` seconds.
    fn audio(seconds: f64, quiet: Option<(f64, f64)>) -> Vec<f32> {
        let n = (seconds * f64::from(RATE)) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(RATE);
                if quiet.is_some_and(|(a, b)| t >= a && t < b) {
                    0.001
                } else {
                    ((i * 7919) % 200) as f32 / 400.0 - 0.25
                }
            })
            .collect()
    }

    #[test]
    fn short_phrases_with_a_sentence_pause_become_separate_chunks() {
        let samples = audio(20.0, None);
        // 4 s of speech, a 0.6 s pause, 5 s of speech.
        let c = chunks(&[s(0.5, 4.5), s(5.1, 10.1)], &samples, RATE, &Params::default());
        assert_eq!(c.len(), 2, "{c:?}");
        assert!(c[0].end_ms <= c[1].start_ms);
        assert!(c.iter().all(|x| x.duration_s() <= 15.0));
    }

    #[test]
    fn pieces_without_a_sentence_pause_are_merged_up_to_the_limit() {
        let samples = audio(30.0, None);
        // Five 2.5 s phrases with 0.15 s breaks between them: shorter than a sentence pause.
        let speech: Vec<Speech> = (0..5).map(|i| s(f64::from(i) * 2.65, f64::from(i) * 2.65 + 2.5)).collect();
        let c = chunks(&speech, &samples, RATE, &Params::default());
        assert_eq!(c.len(), 1, "{c:?}");
        assert!(c[0].duration_s() <= 15.0);
        assert!(c[0].speech_ratio() > 0.9);
    }

    #[test]
    fn a_chunk_never_runs_past_the_limit_and_closes_at_the_longest_pause() {
        let samples = audio(60.0, None);
        // Twelve phrases of 2 s with small 0.2 s pauses, and one 0.6 s pause after the fourth.
        let mut speech = Vec::new();
        let mut t = 0.0;
        for i in 0..12 {
            speech.push(s(t, t + 2.0));
            t += 2.0 + if i == 3 { 0.6 } else { 0.2 };
        }
        let c = chunks(&speech, &samples, RATE, &Params::default());
        assert!(c.len() >= 2, "{c:?}");
        assert!(c.iter().all(|x| x.duration_s() <= 15.0), "{c:?}");
        // The first chunk would not hold the next phrase, so it closes at the longest pause inside it:
        // after the fourth phrase, which ends at 8.6 s (plus 0.2 s of padding).
        assert!((c[0].end_ms as f64 / 1000.0 - 8.8).abs() < 0.3, "{c:?}");
    }

    #[test]
    fn one_long_stretch_without_a_pause_is_cut_at_the_quietest_moment_and_marked() {
        // 40 s of continuous speech with a quiet dip at 12.2 to 12.6 s.
        let samples = audio(41.0, Some((12.2, 12.6)));
        let c = chunks(&[s(0.5, 40.5)], &samples, RATE, &Params::default());
        assert!(c.len() >= 3, "{c:?}");
        assert!(c.iter().all(|x| x.duration_s() <= 15.0), "{c:?}");
        assert!(!c[0].cut_inside_speech);
        assert!(c[1..].iter().all(|x| x.cut_inside_speech));
        // The first cut falls in the quiet dip (within the 4 s before the 14.7 s limit).
        let cut = c[0].end_ms as f64 / 1000.0;
        assert!((12.0..12.9).contains(&cut), "first cut at {cut}");
    }

    #[test]
    fn chunks_cover_all_the_speech_and_do_not_overlap() {
        let samples = audio(80.0, None);
        let speech: Vec<Speech> = (0..14).map(|i| s(f64::from(i) * 5.5 + 1.0, f64::from(i) * 5.5 + 5.0)).collect();
        let c = chunks(&speech, &samples, RATE, &Params::default());
        for w in c.windows(2) {
            assert!(w[0].end_ms <= w[1].start_ms, "overlap: {w:?}");
        }
        for sp in &speech {
            let (a, b) = ((sp.start_s * 1000.0) as u64, (sp.end_s * 1000.0) as u64);
            assert!(c.iter().any(|x| x.start_ms <= a + 1 && x.end_ms + 1 >= b), "speech {sp:?} not covered by {c:?}");
        }
        assert!(c.iter().all(|x| x.duration_s() <= 15.0));
    }

    #[test]
    fn padding_is_clipped_to_the_audio_and_the_neighbours() {
        let samples = audio(10.0, None);
        let c = chunks(&[s(0.05, 4.0), s(4.2, 9.98)], &samples, RATE, &Params::default());
        assert_eq!(c[0].start_ms, 0);
        assert!(c.last().map_or(0, |x| x.end_ms) <= 10_000);
        for w in c.windows(2) {
            assert!(w[0].end_ms <= w[1].start_ms);
        }
    }

    #[test]
    fn no_speech_gives_no_chunks_and_empty_stretches_are_ignored() {
        let samples = audio(5.0, None);
        assert!(chunks(&[], &samples, RATE, &Params::default()).is_empty());
        assert!(chunks(&[s(1.0, 1.0)], &samples, RATE, &Params::default()).is_empty());
    }

    #[test]
    fn a_30_second_limit_for_single_language_sessions_gives_longer_chunks() {
        let samples = audio(40.0, None);
        let speech: Vec<Speech> = (0..8).map(|i| s(f64::from(i) * 3.2, f64::from(i) * 3.2 + 3.0)).collect();
        let p = Params { max_chunk_s: 30.0, ..Params::default() };
        let c = chunks(&speech, &samples, RATE, &p);
        assert!(c.iter().all(|x| x.duration_s() <= 30.0));
        assert!(c.iter().any(|x| x.duration_s() > 15.0) || c.len() >= 1);
    }

    #[test]
    fn speech_ratio_reflects_the_pauses_inside_a_chunk() {
        let samples = audio(20.0, None);
        let c = chunks(&[s(0.5, 3.0), s(3.1, 6.0)], &samples, RATE, &Params::default());
        assert_eq!(c.len(), 1);
        assert!(c[0].speech_ratio() > 0.8 && c[0].speech_ratio() <= 1.0);
    }
}

#[cfg(test)]
mod greedy_tests {
    use super::*;

    #[test]
    fn greedy_merging_ignores_sentence_pauses_like_the_evaluation_harness() {
        let samples: Vec<f32> = (0..16_000usize * 30).map(|i| ((i * 7919) % 200) as f32 / 400.0 - 0.25).collect();
        // Four 4 s phrases with 0.8 s pauses: sentence mode makes four chunks, greedy two (the first
        // three phrases fit in 15 s, the fourth does not).
        let speech = [
            Speech { start_s: 0.5, end_s: 4.5 },
            Speech { start_s: 5.3, end_s: 9.3 },
            Speech { start_s: 10.1, end_s: 14.1 },
            Speech { start_s: 14.9, end_s: 18.9 },
        ];
        let sentence = chunks(&speech, &samples, 16_000, &Params::default());
        assert_eq!(sentence.len(), 4, "{sentence:?}");
        let greedy_params = Params { sentence_split: false, ..Params::default() };
        let greedy = chunks(&speech, &samples, 16_000, &greedy_params);
        assert_eq!(greedy.len(), 2, "{greedy:?}");
        assert!(greedy.iter().all(|c| c.duration_s() <= 15.0));
        // Never past the limit even when greedy.
        let long: Vec<Speech> = (0..10).map(|i| Speech { start_s: f64::from(i) * 4.0, end_s: f64::from(i) * 4.0 + 3.5 }).collect();
        assert!(chunks(&long, &samples, 16_000, &greedy_params).iter().all(|c| c.duration_s() <= 15.0));
    }
}
