//! Text handling for Hindi, Urdu and English output: UTF-8 that is never split, scripts,
//! right-to-left direction and mixed text (AGENTS.md non-negotiable 6: keep each word in its own
//! script; PLAN section 5).

/// Result of turning a chunk's raw model bytes into text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utf8Text {
    pub text: String,
    /// Bytes of an incomplete character at the very end that were dropped.
    pub dropped_tail_bytes: usize,
    /// Bytes that were invalid in the middle and were replaced by U+FFFD.
    pub replaced: bool,
}

impl Utf8Text {
    /// True when the model's output ended inside a character or held invalid bytes.
    pub fn is_broken(&self) -> bool {
        self.dropped_tail_bytes > 0 || self.replaced
    }
}

/// Joins the raw bytes of a chunk's segments and decodes them once.
///
/// whisper.cpp hands out text per segment, and a token boundary can fall inside a multi-byte
/// character (Devanagari and Arabic letters are 3 and 2 bytes), most often where a decode stops
/// at its token limit. Decoding each piece separately turns the halves into U+FFFD. Joining the
/// bytes first keeps every character whole; only a character still incomplete at the very end of
/// the chunk is dropped (and reported).
pub fn join_utf8(parts: &[&[u8]]) -> Utf8Text {
    let mut bytes: Vec<u8> = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
    for p in parts {
        bytes.extend_from_slice(p);
    }
    decode_utf8(&bytes)
}

/// Decodes `bytes`, dropping an incomplete character at the end and replacing invalid bytes.
pub fn decode_utf8(bytes: &[u8]) -> Utf8Text {
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes;
    let mut dropped = 0;
    let mut replaced = false;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                text.push_str(s);
                break;
            }
            Err(e) => {
                let (valid, after) = rest.split_at(e.valid_up_to());
                // `valid` is valid UTF-8 by the error's own definition.
                text.push_str(&String::from_utf8_lossy(valid));
                match e.error_len() {
                    // An incomplete sequence at the end of the input.
                    None => {
                        dropped = after.len();
                        break;
                    }
                    Some(n) => {
                        text.push('\u{FFFD}');
                        replaced = true;
                        rest = &after[n..];
                    }
                }
            }
        }
    }
    Utf8Text {
        text,
        dropped_tail_bytes: dropped,
        replaced,
    }
}

/// Letters per script in a text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScriptCounts {
    pub latin: usize,
    pub devanagari: usize,
    pub arabic: usize,
    /// Other letters (digits and punctuation are not counted).
    pub other: usize,
}

impl ScriptCounts {
    pub fn total(&self) -> usize {
        self.latin + self.devanagari + self.arabic + self.other
    }

    fn share(&self, n: usize) -> f32 {
        if self.total() == 0 {
            0.0
        } else {
            n as f32 / self.total() as f32
        }
    }
}

pub fn is_devanagari(c: char) -> bool {
    matches!(c, '\u{0900}'..='\u{097F}' | '\u{A8E0}'..='\u{A8FF}')
}

pub fn is_arabic(c: char) -> bool {
    matches!(
        c,
        '\u{0600}'..='\u{06FF}'
            | '\u{0750}'..='\u{077F}'
            | '\u{08A0}'..='\u{08FF}'
            | '\u{FB50}'..='\u{FDFF}'
            | '\u{FE70}'..='\u{FEFF}'
    )
}

pub fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic()
        || matches!(c, '\u{00C0}'..='\u{024F}' if c != '\u{00D7}' && c != '\u{00F7}')
}

/// Counts the letters of each script. Combining marks (vowel signs, nukta, Arabic diacritics) belong
/// to the script of the letter they follow; digits, spaces and punctuation are ignored.
pub fn script_counts(text: &str) -> ScriptCounts {
    let mut n = ScriptCounts::default();
    for c in text.chars() {
        if is_devanagari(c) {
            // Digits and the danda are not letters.
            if c.is_alphabetic()
                || matches!(c, '\u{0900}'..='\u{0903}' | '\u{093A}'..='\u{094F}' | '\u{0951}'..='\u{0957}' | '\u{0962}'..='\u{0963}')
            {
                n.devanagari += 1;
            }
        } else if is_arabic(c) {
            if c.is_alphabetic() || matches!(c, '\u{064B}'..='\u{065F}' | '\u{0670}') {
                n.arabic += 1;
            }
        } else if is_latin(c) {
            n.latin += 1;
        } else if c.is_alphabetic() {
            n.other += 1;
        }
    }
    n
}

/// The script a language is written in.
pub fn expected_script(lang: &str) -> Option<Script> {
    match lang {
        "en" => Some(Script::Latin),
        "hi" | "mr" | "ne" | "sa" => Some(Script::Devanagari),
        "ur" | "ar" | "fa" | "ps" | "sd" => Some(Script::Arabic),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Latin,
    Devanagari,
    Arabic,
}

fn count_of(c: &ScriptCounts, s: Script) -> usize {
    match s {
        Script::Latin => c.latin,
        Script::Devanagari => c.devanagari,
        Script::Arabic => c.arabic,
    }
}

/// True when a text with at least `MIN_LETTERS` letters is mostly in a script other than the one
/// its language is written in: Hindi in Latin letters, English in Devanagari, Urdu in Devanagari.
/// A Hindi sentence may carry English words (Hinglish), so Hindi and Urdu only fail when less than
/// 20% of their letters are in their own script; English fails below 50%.
pub fn script_contradicts(lang: &str, counts: &ScriptCounts) -> bool {
    const MIN_LETTERS: usize = 4;
    let Some(script) = expected_script(lang) else {
        return false;
    };
    if counts.total() < MIN_LETTERS {
        return false;
    }
    let own = counts.share(count_of(counts, script));
    let limit = if script == Script::Latin { 0.5 } else { 0.2 };
    own < limit
}

/// More than one script in the text: the minority script has at least 2 letters and 5% of all.
pub fn is_mixed(counts: &ScriptCounts) -> bool {
    let all = [counts.latin, counts.devanagari, counts.arabic, counts.other];
    let total = counts.total();
    if total == 0 {
        return false;
    }
    let mut sorted = all;
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let second = sorted[1];
    second >= 2 && second as f32 / total as f32 >= 0.05
}

/// How a renderer should lay the text out: `rtl` for Urdu-style text, `auto` for mixed text, `ltr`
/// otherwise.
pub fn direction(counts: &ScriptCounts) -> &'static str {
    if is_mixed(counts) {
        "auto"
    } else if counts.arabic > counts.latin + counts.devanagari + counts.other {
        "rtl"
    } else {
        "ltr"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HINDI: &str = "नमस्ते दोस्तों, आज हम स्टेलमार के बारे में बात करेंगे।";
    const URDU: &str = "یہ ایک ٹیسٹ ہے اور ہم اردو میں بات کر رہے ہیں۔";

    #[test]
    fn a_character_cut_between_two_segments_is_joined_not_replaced() {
        // Cut the Devanagari string at every byte: the two halves, joined, are the original, and
        // decoding the halves one by one is what produces U+FFFD.
        let bytes = HINDI.as_bytes();
        for cut in 0..=bytes.len() {
            let joined = join_utf8(&[&bytes[..cut], &bytes[cut..]]);
            assert_eq!(joined.text, HINDI, "cut at byte {cut}");
            assert!(!joined.is_broken(), "cut at byte {cut}");
        }
        // Control: separate lossy decoding does break some cuts.
        let broken = (0..=bytes.len()).filter(|&cut| {
            let a = String::from_utf8_lossy(&bytes[..cut]);
            let b = String::from_utf8_lossy(&bytes[cut..]);
            format!("{a}{b}") != HINDI
        });
        assert!(broken.count() > 0);
    }

    #[test]
    fn urdu_is_kept_whole_at_every_cut_too() {
        let bytes = URDU.as_bytes();
        for cut in 0..=bytes.len() {
            assert_eq!(
                join_utf8(&[&bytes[..cut], &bytes[cut..]]).text,
                URDU,
                "cut at byte {cut}"
            );
        }
    }

    #[test]
    fn a_character_still_incomplete_at_the_end_is_dropped_and_reported() {
        let bytes = HINDI.as_bytes();
        // Drop the last byte of the final danda (3 bytes): 2 bytes remain of an incomplete character.
        let cut = &bytes[..bytes.len() - 1];
        let t = decode_utf8(cut);
        assert_eq!(t.dropped_tail_bytes, 2);
        assert!(!t.replaced);
        assert!(t.is_broken());
        assert_eq!(t.text, HINDI.trim_end_matches('।'));
        assert!(!t.text.contains('\u{FFFD}'));
    }

    #[test]
    fn invalid_bytes_in_the_middle_are_replaced_and_reported() {
        let t = decode_utf8(&[b'a', 0xFF, b'b']);
        assert_eq!(t.text, "a\u{FFFD}b");
        assert!(t.replaced);
        assert_eq!(t.dropped_tail_bytes, 0);
    }

    #[test]
    fn empty_input_is_empty_text() {
        let t = join_utf8(&[]);
        assert_eq!(t.text, "");
        assert!(!t.is_broken());
    }

    #[test]
    fn scripts_are_counted_per_letter() {
        let h = script_counts(HINDI);
        assert!(h.devanagari > 30 && h.latin == 0 && h.arabic == 0, "{h:?}");
        let u = script_counts(URDU);
        assert!(u.arabic > 25 && u.devanagari == 0 && u.latin == 0, "{u:?}");
        let e = script_counts("Welcome to STELMAR 2027, everyone.");
        assert!(e.latin > 20 && e.devanagari == 0, "{e:?}");
        // Digits and punctuation are not letters.
        assert_eq!(script_counts("2027, ... 12:30").total(), 0);
    }

    #[test]
    fn hinglish_is_mixed_and_a_single_foreign_letter_is_not() {
        let mixed = script_counts("आज हम STELMAR के session में बात करेंगे");
        assert!(is_mixed(&mixed));
        assert_eq!(direction(&mixed), "auto");
        assert!(!is_mixed(&script_counts("यह पूरा वाक्य हिंदी में है")));
        assert!(!is_mixed(&script_counts("यह पूरा वाक्य हिंदी में है x")));
    }

    #[test]
    fn urdu_is_right_to_left_and_english_is_not() {
        assert_eq!(direction(&script_counts(URDU)), "rtl");
        assert_eq!(direction(&script_counts("plain English text")), "ltr");
        assert_eq!(direction(&script_counts("")), "ltr");
    }

    #[test]
    fn output_in_the_wrong_script_is_caught() {
        // Hindi speech written in Latin letters, English speech in Devanagari, Urdu in Devanagari.
        assert!(script_contradicts(
            "hi",
            &script_counts("namaste doston aaj hum baat karenge")
        ));
        assert!(script_contradicts(
            "en",
            &script_counts("नमस्ते दोस्तों आज हम बात करेंगे")
        ));
        assert!(script_contradicts("ur", &script_counts("यह एक टेस्ट है")));
        // Right scripts, including Hindi with English words.
        assert!(!script_contradicts("hi", &script_counts(HINDI)));
        assert!(!script_contradicts(
            "hi",
            &script_counts("आज हम STELMAR के session में बात करेंगे")
        ));
        assert!(!script_contradicts("ur", &script_counts(URDU)));
        assert!(!script_contradicts(
            "en",
            &script_counts("Welcome everyone")
        ));
        // Too short to judge, and a language with no expected script.
        assert!(!script_contradicts("hi", &script_counts("ok")));
        assert!(!script_contradicts(
            "fr",
            &script_counts("bonjour tout le monde")
        ));
    }
}
