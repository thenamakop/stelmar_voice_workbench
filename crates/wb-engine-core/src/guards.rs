//! Guards against hallucinated output (PLAN section 6, "Guards").
//!
//! A chunk is dropped (with an event) when the VAD speech ratio is under 0.3 or when `no_speech_prob` is
//! over 0.6 with an average log-probability under -1.0. A chunk whose text looks like a loop (the
//! script-independent compression ratio over `COMPRESSION_ABOVE`) or that reached the token limit is
//! retried before anything is given up (`wb-engine`'s ladder, PLAN section 6, changed 10 Nov 2026).
//! Repeated n-grams, known silence phrases and output in the wrong script are flagged for review and
//! never reach captions.
//!
//! Why the compression ratio is not whisper's byte ratio: UTF-8 spends three bytes on a Devanagari or
//! Arabic-script letter, two of them nearly constant, so ordinary Hindi compresses far better than
//! English by byte count. The reference transcript of one CoSHE-Eval clip scores 2.43 on whisper's
//! measure, and real speech in Hindi and Hinglish went over 2.4 and was dropped. The measure here is
//! computed per character (`guard-calibration.txt` has the calibration).

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
/// Loop threshold of `compression_ratio`, calibrated 10 Nov 2026 (`docs/evidence/phase-2/guard-calibration.txt`):
/// the geometric middle of the highest ratio of any reference transcript (2.55, a whole CoSHE-Eval utterance;
/// no 25-word window passes 2.0) and the lowest known loop (2.74). Do not change it without re-running the
/// calibration.
pub const COMPRESSION_ABOVE: f32 = 2.64;

/// Coverage check (PLAN section 6, 11 Nov 2026): the floor of letters and digits per second of VAD
/// speech below which a chunk's text is too short for the speech it holds, by language. A decode like
/// that is not a loop and the model is confident of it, so no other guard sees it (one 14 s Hinglish
/// chunk came out as 43 characters). The values come from `scripts/gates/coverage_calibration.py`
/// (`docs/evidence/phase-2/coverage-calibration.txt`, 11 Nov 2026: 0.9 x the lowest reference rate of the language); a language with no entry is not checked.
pub const COVERAGE_FLOOR: &[(&str, f32)] = &[("en", 5.79), ("hi", 5.03), ("ur", 7.70)];

/// Chunks with less speech than this are not judged: their rate varies too much.
pub const COVERAGE_MIN_SPEECH_S: f64 = 3.0;

pub fn coverage_floor(lang: &str) -> Option<f32> {
    COVERAGE_FLOOR
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, f)| *f)
}

/// Letters and digits in `text` (the unit of the coverage rate; spaces and punctuation do not count).
pub fn letter_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphanumeric()).count()
}

/// Letters and digits per second of speech, or `None` when the chunk holds too little speech to judge.
pub fn chars_per_speech_second(text: &str, speech_s: f64) -> Option<f32> {
    if speech_s < COVERAGE_MIN_SPEECH_S {
        return None;
    }
    Some((letter_count(text) as f64 / speech_s) as f32)
}

/// True when `text` is far below the normal rate for `lang` given `speech_s` seconds of speech.
pub fn low_coverage(text: &str, lang: &str, speech_s: f64) -> bool {
    match (coverage_floor(lang), chars_per_speech_second(text, speech_s)) {
        (Some(floor), Some(rate)) => rate < floor,
        _ => false,
    }
}

fn zlib_ratio(raw: &[u8]) -> f32 {
    if raw.is_empty() {
        return 1.0;
    }
    let mut enc = ZlibEncoder::new(Vec::with_capacity(raw.len()), Compression::default());
    if enc.write_all(raw).is_err() {
        return 1.0;
    }
    match enc.finish() {
        Ok(out) if !out.is_empty() => raw.len() as f32 / out.len() as f32,
        _ => 1.0,
    }
}

/// Whisper's own measure: the UTF-8 length of the text divided by its zlib-compressed length. Kept only
/// to compare with `compression_ratio` in the calibration; it is script-dependent.
pub fn compression_ratio_utf8(text: &str) -> f32 {
    zlib_ratio(text.as_bytes())
}

/// Script-independent loop measure: characters per compressed byte. Each distinct character is first
/// mapped to one byte (in order of first appearance), so a Devanagari or Urdu letter costs one byte
/// like a Latin one and the ratio means the same in every script; the mapping is one-to-one, so
/// repetition compresses exactly as before. Text with more than 256 distinct characters (not seen in
/// English, Hindi or Urdu) falls back to the UTF-8 measure.
pub fn compression_ratio(text: &str) -> f32 {
    let mut map: std::collections::HashMap<char, u8> = std::collections::HashMap::new();
    let mut raw: Vec<u8> = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let next = map.len();
        match map.get(&ch) {
            Some(b) => raw.push(*b),
            None => {
                let Ok(b) = u8::try_from(next) else {
                    return compression_ratio_utf8(text);
                };
                map.insert(ch, b);
                raw.push(b);
            }
        }
    }
    zlib_ratio(&raw)
}

/// Whether the chunk should be given up on, and why. `Compression` here means "looks like a loop": the
/// caller retries it before dropping anything.
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
            drop_reason(&m(-0.3, 0.1, 2.7, 0.9)),
            Some(DropReason::Compression)
        );
        assert_eq!(
            drop_reason(&m(-0.3, 0.1, COMPRESSION_ABOVE, 0.9)),
            None,
            "the threshold itself is not over the threshold"
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
    fn coverage_counts_letters_per_second_of_speech_and_skips_short_chunks() {
        // 40 letters in 10 s of speech: 4 per second.
        let t = "abcdefghij abcdefghij abcdefghij abcdefghij";
        assert_eq!(letter_count(t), 40);
        let r = chars_per_speech_second(t, 10.0).unwrap_or(0.0);
        assert!((r - 4.0).abs() < 1e-6, "{r}");
        // Under 3 s of speech nothing is judged.
        assert_eq!(chars_per_speech_second(t, 2.9), None);
        // Devanagari letters and vowel signs count as letters; punctuation does not.
        assert!(letter_count("नमस्ते, दोस्तों।") >= 10);
        assert_eq!(letter_count(" , . ! । "), 0);
        // A language without a floor is never flagged.
        assert!(!low_coverage("a", "xx", 30.0));
        assert!(!low_coverage("", "en", 30.0) || coverage_floor("en").is_some());
    }

    #[test]
    fn the_calibrated_floors_flag_a_short_decode_and_not_a_normal_one() {
        // The known case (audio_1444 chunk 1): 3.34 letters per second of speech, floor 5.03 for hi.
        let short = "a".repeat(46); // 46 letters in 13.8 s of speech = 3.33 per second
        assert!(low_coverage(&short, "hi", 13.8));
        // A normal rate (the median of the references is about 10 to 14 per second) is not flagged.
        let normal = "a".repeat(140);
        assert!(!low_coverage(&normal, "hi", 13.8));
        assert!(!low_coverage(&normal, "en", 13.8));
        assert!(!low_coverage(&normal, "ur", 13.8));
        // The same short decode in a chunk with under 3 s of speech is not judged.
        assert!(!low_coverage("abc", "hi", 2.0));
        assert_eq!(coverage_floor("hi"), Some(5.03));
    }

    #[test]
    fn the_measure_means_the_same_in_every_script() {
        // Ordinary speech with repeated words (as in audio_1444: Hindi with a repeated phrase) is not a loop
        // in any script, although the UTF-8 byte measure puts Devanagari far above English.
        let hindi = "हम हम आज आज यह बात बात बता रहे हैं कि सत्र में सत्र में कौन कौन बोलेगा और कब बोलेगा";
        let urdu = "ہم ہم آج آج یہ بات بات بتا رہے ہیں کہ سیشن میں سیشن میں کون کون بولے گا اور کب بولے گا";
        let english = "we we today today this matter matter is what we are telling you that in the session in the session who will speak and when";
        for t in [hindi, urdu, english] {
            assert!(
                compression_ratio(t) <= COMPRESSION_ABOVE,
                "{t}: {}",
                compression_ratio(t)
            );
        }
        assert!(compression_ratio_utf8(hindi) > compression_ratio(hindi));
        // A real loop is over the threshold in each script.
        for unit in ["धन्यवाद आप सभी का ", "شکریہ آپ سب کا ", "thank you all of you "] {
            let looped = unit.repeat(12);
            assert!(
                compression_ratio(&looped) > COMPRESSION_ABOVE,
                "{unit}: {}",
                compression_ratio(&looped)
            );
        }
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
