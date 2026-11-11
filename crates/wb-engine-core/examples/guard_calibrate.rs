//! Applies the engine's own guard measures to a list of texts, for calibrating the loop threshold
//! (`scripts/gates/guard_calibration.py`). Development only; not part of any build we ship.
//!
//! Input (stdin): one text per line as `id<TAB>set<TAB>text`. Output (stdout): `id<TAB>set<TAB>chars<TAB>
//! words<TAB>ratio<TAB>ratio_utf8<TAB>repetition`, where `ratio` is `guards::compression_ratio` (characters per
//! compressed byte, script-independent), `ratio_utf8` is whisper's byte measure and `repetition` is 1 when
//! `guards::has_repetition` fires.

use std::io::BufRead;

use wb_engine_core::guards;

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let mut parts = line.splitn(3, '\t');
        let (Some(id), Some(set), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        println!(
            "{id}\t{set}\t{}\t{}\t{:.4}\t{:.4}\t{}\t{}",
            text.chars().count(),
            text.split_whitespace().count(),
            guards::compression_ratio(text),
            guards::compression_ratio_utf8(text),
            u8::from(guards::has_repetition(text)),
            guards::letter_count(text)
        );
    }
}
