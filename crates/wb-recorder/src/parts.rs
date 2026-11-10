//! `parts.json`: the index of closed parts that Phase 4's post-processing cursor and Phase 6's
//! player follow. Written atomically; rebuilt from the files after a crash.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::util::{read_json, sha256_file, write_json_atomic};
use crate::wav::{frames_to_ms, inspect};

pub const PARTS_FILE: &str = "parts.json";
pub const AUDIO_DIR: &str = "audio";

/// One closed part. Times are milliseconds on the session's audio clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartRecord {
    pub part: u32,
    /// Path relative to the session folder, for example `audio/part-0001.wav`.
    pub file: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub sha256: String,
    /// The file's header does not match its length (a write failed while the part was closing).
    /// `frames` and `duration_ms` come from the length; recovery rewrites the header.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_repair: bool,
}

impl PartRecord {
    pub fn end_ms(&self) -> u64 {
        self.start_ms + self.duration_ms
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartsIndex {
    pub version: u32,
    pub parts: Vec<PartRecord>,
}

impl PartsIndex {
    pub fn new() -> Self {
        Self {
            version: 1,
            parts: Vec::new(),
        }
    }

    /// Audio-clock end of the last part (where the next part starts).
    pub fn end_ms(&self) -> u64 {
        self.parts.last().map_or(0, PartRecord::end_ms)
    }

    /// Number the next part gets.
    pub fn next_part(&self) -> u32 {
        self.parts.last().map_or(1, |p| p.part + 1)
    }
}

/// File name of part `n`, for example `part-0001.wav`.
pub fn part_file_name(n: u32) -> String {
    format!("part-{n:04}.wav")
}

/// Relative path of part `n` inside the session folder.
pub fn part_rel_path(n: u32) -> String {
    format!("{AUDIO_DIR}/{}", part_file_name(n))
}

/// Part number from a file name like `part-0012.wav`.
pub fn parse_part_number(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("part-")?.strip_suffix(".wav")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub fn parts_path(session_dir: &Path) -> PathBuf {
    session_dir.join(PARTS_FILE)
}

/// Loads `parts.json`; a missing file is an empty index.
pub fn load(session_dir: &Path) -> Result<PartsIndex, Error> {
    let path = parts_path(session_dir);
    if !path.exists() {
        return Ok(PartsIndex::new());
    }
    read_json(&path).map_err(|e| Error::io(&path, e))
}

pub fn save(session_dir: &Path, index: &PartsIndex) -> Result<(), Error> {
    let path = parts_path(session_dir);
    write_json_atomic(&path, index).map_err(|e| Error::io(&path, e))
}

/// Builds the record for a closed part file. `start_ms` is where the previous part ended.
pub fn record_for_file(session_dir: &Path, part: u32, start_ms: u64) -> Result<PartRecord, Error> {
    let rel = part_rel_path(part);
    let path = session_dir.join(AUDIO_DIR).join(part_file_name(part));
    let info = inspect(&path)?;
    let sha256 = sha256_file(&path).map_err(|e| Error::io(&path, e))?;
    Ok(PartRecord {
        part,
        file: rel,
        start_ms,
        duration_ms: frames_to_ms(info.frames_on_disk, info.sample_rate),
        frames: info.frames_on_disk,
        sample_rate: info.sample_rate,
        channels: info.channels,
        sha256,
        needs_repair: !info.header_matches_length(),
    })
}

/// Part numbers present in `audio/`, ascending.
pub fn part_numbers_on_disk(session_dir: &Path) -> Result<Vec<u32>, Error> {
    let audio = session_dir.join(AUDIO_DIR);
    if !audio.exists() {
        return Ok(Vec::new());
    }
    let mut numbers = Vec::new();
    for entry in std::fs::read_dir(&audio).map_err(|e| Error::io(&audio, e))? {
        let entry = entry.map_err(|e| Error::io(&audio, e))?;
        if let Some(n) = entry.file_name().to_str().and_then(parse_part_number) {
            numbers.push(n);
        }
    }
    numbers.sort_unstable();
    Ok(numbers)
}

/// Rebuilds the index from the part files alone: each part starts where the previous one ended.
/// Headers should be repaired first (`recovery` does this).
pub fn rebuild_from_folder(session_dir: &Path) -> Result<PartsIndex, Error> {
    let mut index = PartsIndex::new();
    let mut start_ms = 0;
    for n in part_numbers_on_disk(session_dir)? {
        let rec = record_for_file(session_dir, n, start_ms)?;
        start_ms = rec.end_ms();
        index.parts.push(rec);
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crate::wav::PartWriter;

    fn write_part(
        dir: &Path,
        n: u32,
        frames: usize,
        rate: u32,
        channels: u16,
    ) -> Result<(), Error> {
        let audio = dir.join(AUDIO_DIR);
        std::fs::create_dir_all(&audio).map_err(|e| Error::io(&audio, e))?;
        let mut w = PartWriter::create(&audio.join(part_file_name(n)), rate, channels)?;
        let samples: Vec<f32> = (0..frames * usize::from(channels))
            .map(|i| ((i % 200) as f32 - 100.0) / 400.0)
            .collect();
        w.write_f32(&samples)?;
        w.finish()?;
        Ok(())
    }

    #[test]
    fn names_round_trip() {
        assert_eq!(part_file_name(1), "part-0001.wav");
        assert_eq!(part_rel_path(12), "audio/part-0012.wav");
        assert_eq!(parse_part_number("part-0012.wav"), Some(12));
        assert_eq!(parse_part_number("part-12345.wav"), Some(12345));
        assert_eq!(parse_part_number("part-.wav"), None);
        assert_eq!(parse_part_number("part-00a1.wav"), None);
        assert_eq!(parse_part_number("part-0001.wav.tmp"), None);
    }

    #[test]
    fn rebuild_matches_the_index_written_while_recording() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = TempDir::new("parts-rebuild")?;
        let d = dir.path();
        // Three parts as the recorder would close them: 2.5 s, 1 s (a pause), then 0.75 s at a
        // different rate after a device reconnect.
        write_part(d, 1, 120_000, 48_000, 2)?;
        write_part(d, 2, 48_000, 48_000, 2)?;
        write_part(d, 3, 33_075, 44_100, 1)?;
        // A stray file that is not a part.
        std::fs::write(d.join(AUDIO_DIR).join("notes.txt"), "x")?;

        let mut live = PartsIndex::new();
        for n in 1..=3 {
            let rec = record_for_file(d, n, live.end_ms())?;
            live.parts.push(rec);
        }
        save(d, &live)?;

        let rebuilt = rebuild_from_folder(d)?;
        assert_eq!(rebuilt, live);
        assert_eq!(load(d)?, live);

        let p = &rebuilt.parts;
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].start_ms, p[0].duration_ms), (0, 2_500));
        assert_eq!((p[1].start_ms, p[1].duration_ms), (2_500, 1_000));
        assert_eq!(
            (
                p[2].start_ms,
                p[2].duration_ms,
                p[2].sample_rate,
                p[2].channels
            ),
            (3_500, 750, 44_100, 1)
        );
        assert_eq!(p[0].sha256, sha256_file(&d.join("audio/part-0001.wav"))?);
        assert_eq!(rebuilt.next_part(), 4);
        assert_eq!(rebuilt.end_ms(), 4_250);
        Ok(())
    }

    #[test]
    fn rebuild_of_an_empty_session_is_empty() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new("parts-empty")?;
        assert_eq!(rebuild_from_folder(dir.path())?, PartsIndex::new());
        assert_eq!(load(dir.path())?.next_part(), 1);
        Ok(())
    }
}
