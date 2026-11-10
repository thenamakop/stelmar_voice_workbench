//! Guards against hallucinated output (PLAN section 6, "Guards").
//!
//! A segment is dropped when the VAD speech ratio is under 0.3, when `no_speech_prob` is over 0.6
//! with an average log-probability under -1.0, or when the compression ratio is over 2.4. Repeated
//! n-grams, known silence phrases and output in the wrong script are flagged for review and never
//! reach captions.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use wb_protocol::SegmentFlag;

use crate::text::{ScriptCounts, script_contradicts};

/// What the model said about a decoded chunk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub avg_logprob: f32,
    pub no_speech_prob: f32,
    pub compression_ratio: f32,
    /// Share of the chunk the VAD called speech, 0 to 1.
    pub speech_ratio: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// Under 30% of the chunk is speech.
    LittleSpeech,
    /// The model thinks it is not speech and is unsure of what it wrote.
    NoSpeech,
    /// Text that compresses far better than speech does: a loop.
    Compression,
}

impl DropReason {
    pub fn flag(self) -> SegmentFlag {
        match self {
            Self::LittleSpeech | Self::NoSpeech => SegmentFlag::NoSpeech,
            Self::Compression => SegmentFlag::CompressionRatio,
        }
    }
}

pub const MIN_SPEECH_RATIO: f32 = 0.3;
pub const NO_SPEECH_ABOVE: f32 = 0.6;
pub const LOGPROB_BELOW: f32 = -1.0;
pub const COMPRESSION_ABOVE: f32 = 2.4;

/// Length of the text divided by its DEFLATE-compressed length (zlib, as whisper's own guard).
pub fn compression_ratio(text: &str) -> f32 {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return 1.0;
    }
    let mut enc = ZlibEncoder::new(Vec::with_capacity(bytes.len()), Compression::default());
    if enc.write_all(bytes).is_err() {
        return 1.0;
    }
    match enc.finish() {
        Ok(out) if !out.is_empty() => bytes.len() as f32 / out.len() as f32,
        _ => 1.0,
    }
}

/// Whether the chunk should be dropped, and why.
pub fn drop_reason(m: &Metrics) -> Option<DropReason> {
    if m.speech_ratio < MIN_SPEECH_RATIO {
        Some(DropReason::LittleSpeech)
    } else if m.no_speech_prob > NO_SPEECH_ABOVE && m.avg_logprob < LOGPROB_BELOW {
        Some(DropReason::NoSpeech)
    } else if m.compression_ratio > COMPRESSION_ABOVE {
        Some(DropReason::Compression)
    } else {
        None
    }
}

/// True when some run of 1 to 4 words repeats back to back often enough to be a loop: 5 times for
/// phrases, 8 for single words (a speaker may say "no no no").
pub fn has_repetition(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    for n in 1..=4usize {
        if words.len() < n * 4 {
            continue;
        }
        let need = if n == 1 { 8 } else { 5 };
        let mut run = 1;
        let mut i = 0;
        while i + 2 * n <= words.len() {
            if words[i..i + n] == words[i + n..i + 2 * n] {
                run += 1;
                if run >= need {
                    return true;
                }
            } else {
                run = 1;
            }
            i += n;
        }
    }
    false
}

/// Phrases whisper models write over silence or music (lower case, matched as whole text after
/// trimming punctuation).
const SILENCE_PHRASES: &[&str] = &[
    "thanks for watching",
    "thank you for watching",
    "thank you so much for watching",
    "please subscribe",
    "like and subscribe",
    "subscribe to my channel",
    "subtitles by the amara.org community",
    "see you in the next video",
    "देखने के लिए धन्यवाद",
    "वीडियो देखने के लिए धन्यवाद",
    "सब्सक्राइब करें",
    "चैनल को सब्सक्राइब करें",
    "دیکھنے کے لیے شکریہ",
];

/// Credits whisper models write over music or silence; any text that starts with one is a hallucination.
const SILENCE_PREFIXES: &[&str] = &[
    "subtitles by",
    "subtitled by",
    "transcribed by",
    "translated by",
];

pub fn is_silence_phrase(text: &str) -> bool {
    let t: String = text
        .trim()
        .trim_matches(|c: char| {
            c.is_ascii_punctuation() || c == '।' || c == '۔' || c.is_whitespace()
        })
        .to_lowercase();
    !t.is_empty()
        && (SILENCE_PHRASES.contains(&t.as_str())
            || SILENCE_PREFIXES.iter().any(|p| t.starts_with(p)))
}

/// Flags to attach to a kept segment (they are not reasons to drop it).
pub fn flags(text: &str, lang: &str, counts: &ScriptCounts) -> Vec<SegmentFlag> {
    let mut out = Vec::new();
    if has_repetition(text) {
        out.push(SegmentFlag::Repetition);
    }
    if is_silence_phrase(text) {
        out.push(SegmentFlag::SilencePhrase);
    }
    if script_contradicts(lang, counts) {
        out.push(SegmentFlag::WrongScript);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::script_counts;

    fn m(avg: f32, nsp: f32, cr: f32, speech: f32) -> Metrics {
        Metrics {
            avg_logprob: avg,
            no_speech_prob: nsp,
            compression_ratio: cr,
            speech_ratio: speech,
        }
    }

    #[test]
    fn the_three_drop_rules_match_the_plan() {
        assert_eq!(drop_reason(&m(-0.3, 0.1, 1.5, 0.9)), None);
        assert_eq!(
            drop_reason(&m(-0.3, 0.1, 1.5, 0.29)),
            Some(DropReason::LittleSpeech)
        );
        assert_eq!(
            drop_reason(&m(-0.3, 0.1, 1.5, 0.3)),
            None,
            "0.3 is not under 0.3"
        );
        // no-speech needs both conditions.
        assert_eq!(
            drop_reason(&m(-1.2, 0.7, 1.5, 0.9)),
            Some(DropReason::NoSpeech)
        );
        assert_eq!(drop_reason(&m(-0.5, 0.7, 1.5, 0.9)), None);
        assert_eq!(drop_reason(&m(-1.2, 0.5, 1.5, 0.9)), None);
        assert_eq!(
            drop_reason(&m(-0.3, 0.1, 2.5, 0.9)),
            Some(DropReason::Compression)
        );
        assert_eq!(
            drop_reason(&m(-0.3, 0.1, 2.4, 0.9)),
            None,
            "2.4 is not over 2.4"
        );
    }

    #[test]
    fn a_loop_compresses_better_than_speech() {
        let speech = "Welcome to the opening session of STELMAR 2027. Today we will talk about how the programme is organised and who will speak in each hall.";
        let looped = "thank you thank you thank you thank you thank you thank you thank you thank you thank you thank you thank you thank you";
        assert!(
            compression_ratio(speech) < 2.0,
            "{}",
            compression_ratio(speech)
        );
        assert!(
            compression_ratio(looped) > COMPRESSION_ABOVE,
            "{}",
            compression_ratio(looped)
        );
        assert_eq!(compression_ratio(""), 1.0);
        // Hindi text compresses like speech, not like a loop.
        let hindi =
            "नमस्ते दोस्तों, आज हम स्टेलमार के उद्घाटन सत्र के बारे में बात करेंगे और बताएंगे कि हर हॉल में कौन बोलेगा।";
        assert!(
            compression_ratio(hindi) < COMPRESSION_ABOVE,
            "{}",
            compression_ratio(hindi)
        );
    }

    #[test]
    fn repeated_phrases_are_found_and_ordinary_repeats_are_not() {
        assert!(has_repetition(
            "so then so then so then so then so then we left"
        ));
        assert!(has_repetition("no no no no no no no no stop"));
        assert!(has_repetition("यह है यह है यह है यह है यह है"));
        assert!(!has_repetition("no no no that is not what I said"));
        assert!(!has_repetition(
            "we went to the market and then we went to the station and then home"
        ));
        assert!(!has_repetition(""));
    }

    #[test]
    fn known_silence_phrases_are_recognised_in_three_languages() {
        for t in [
            "Thanks for watching!",
            " thank you for watching. ",
            "Subtitles by the Amara.org community",
            "देखने के लिए धन्यवाद।",
            "دیکھنے کے لیے شکریہ۔",
        ] {
            assert!(is_silence_phrase(t), "{t}");
        }
        for t in [
            "Thanks for coming to the session",
            "Please subscribe to the newsletter at the desk",
            "",
            "।",
        ] {
            assert!(!is_silence_phrase(t), "{t}");
        }
    }

    #[test]
    fn flags_cover_loops_silence_phrases_and_the_wrong_script() {
        let t = "namaste doston aaj hum baat karenge";
        assert_eq!(
            flags(t, "hi", &script_counts(t)),
            vec![SegmentFlag::WrongScript]
        );
        let loop_t = "go go go go go go go go go";
        assert_eq!(
            flags(loop_t, "en", &script_counts(loop_t)),
            vec![SegmentFlag::Repetition]
        );
        let ok = "Welcome everyone";
        assert!(flags(ok, "en", &script_counts(ok)).is_empty());
        let both = "Thanks for watching";
        assert_eq!(
            flags(both, "en", &script_counts(both)),
            vec![SegmentFlag::SilencePhrase]
        );
    }

    #[test]
    fn drop_reasons_map_to_protocol_flags() {
        assert_eq!(
            DropReason::Compression.flag(),
            SegmentFlag::CompressionRatio
        );
        assert_eq!(DropReason::LittleSpeech.flag(), SegmentFlag::NoSpeech);
    }
}
