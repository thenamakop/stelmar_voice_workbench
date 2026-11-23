//! The single writer of transcript segments (AGENTS.md: only the app writes transcript files; every
//! committed segment is appended to its revision's JSONL and fsynced before it is shown anywhere).
//!
//! Resumable jobs: progress is a `committed_until_ms` cursor, the end of the last committed segment.
//! After an engine is killed the next one starts at the cursor (`resume_from_ms`) and skips every
//! chunk that ends at or before it, so nothing is repeated and nothing committed is lost. The
//! committer also refuses a segment that starts before the cursor (a repeat), as a second line of
//! defence, and counts it.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use wb_protocol::SegmentEvent;

/// The line that closes a chunk in a staged file: `{"chunk_end_ms":N}`. Everything above the last marker is
/// committed; anything below it is the start of a chunk that was being written and is cut off on reopening.
const MARKER_KEY: &str = "chunk_end_ms";

#[derive(Debug)]
pub struct Committer {
    path: PathBuf,
    file: File,
    committed_until_ms: u64,
    segments: u64,
    repeats_refused: u64,
    /// Staged mode: every `commit_chunk` ends with a marker line, and `open_staged` restores the cursor from it.
    markers: bool,
}

/// One line of a staged file.
enum StagedLine {
    Segment(SegmentEvent),
    Marker(u64),
    Bad(String),
}

fn parse_staged_line(line: &[u8]) -> StagedLine {
    match serde_json::from_slice::<serde_json::Value>(line) {
        Ok(v) if v.get(MARKER_KEY).is_some() && v.get("start_ms").is_none() => {
            match v.get(MARKER_KEY).and_then(serde_json::Value::as_u64) {
                Some(n) => StagedLine::Marker(n),
                None => StagedLine::Bad(format!("{MARKER_KEY} is not a number")),
            }
        }
        Ok(v) => match serde_json::from_value::<SegmentEvent>(v) {
            Ok(seg) => StagedLine::Segment(seg),
            Err(e) => StagedLine::Bad(e.to_string()),
        },
        Err(e) => StagedLine::Bad(e.to_string()),
    }
}

impl Committer {
    /// Opens (creating) a JSONL file. An existing file is read to restore the cursor and the
    /// count. A torn LAST line (a write cut short by a crash) is cut off so appending resumes on a
    /// clean line; a last line that parses but lacks its newline gets one. A line that does not
    /// parse with more lines after it is NOT cut off (that would delete segments that were already
    /// committed and fsynced): the file is refused with an `InvalidData` error.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut committed = 0u64;
        let mut segments = 0u64;
        let mut cut_at: Option<u64> = None;
        let mut needs_newline = false;
        if path.is_file() {
            let data = std::fs::read(path)?;
            let mut offset = 0usize;
            while offset < data.len() {
                let (line, next, terminated) = match data[offset..].iter().position(|b| *b == b'\n')
                {
                    Some(i) => (&data[offset..offset + i], offset + i + 1, true),
                    None => (&data[offset..], data.len(), false),
                };
                if line.iter().all(u8::is_ascii_whitespace) {
                    offset = next; // a blank line is not a segment and not damage
                    continue;
                }
                match serde_json::from_slice::<SegmentEvent>(line) {
                    Ok(seg) => {
                        committed = committed.max(seg.end_ms);
                        segments += 1;
                        needs_newline = !terminated;
                    }
                    Err(e) => {
                        if data[next..].iter().all(u8::is_ascii_whitespace) {
                            cut_at = Some(offset as u64); // the last line is torn
                        } else {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!(
                                    "{}: a line at byte {offset} is not a segment ({e}) and more lines follow it; refusing to cut off committed segments",
                                    path.display()
                                ),
                            ));
                        }
                        break;
                    }
                }
                offset = next;
            }
            if let Some(keep) = cut_at {
                let f = OpenOptions::new().write(true).open(path)?;
                f.set_len(keep)?;
                f.sync_all()?;
                needs_newline = false;
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if needs_newline {
            file.write_all(b"\n")?;
            file.sync_data()?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            file,
            committed_until_ms: committed,
            segments,
            repeats_refused: 0,
            markers: false,
        })
    }

    /// Opens (creating) a STAGED file, the one a work unit writes before its segments are committed to the
    /// revision. Each chunk ends with a marker line holding the cursor, so the cursor is restored exactly
    /// (an empty chunk and the end of a unit included: [`Committer::open`] cannot know them) and a chunk that
    /// was cut short by a crash or a power loss, even half written, is cut off whole, so the chunk is decoded
    /// again and the join never shows a seam. A bad line above the last marker refuses the file (nothing is
    /// cut); anything after the last marker is the chunk that was being written and is removed.
    pub fn open_staged(path: &Path) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut committed = 0u64;
        let mut segments = 0u64;
        let mut keep = 0u64;
        let mut missing_newline = false;
        if path.is_file() {
            let data = std::fs::read(path)?;
            // Split into lines, remembering where each ends.
            let mut lines: Vec<(usize, usize, StagedLine, bool)> = Vec::new();
            let mut offset = 0usize;
            while offset < data.len() {
                let (end, next, terminated) = match data[offset..].iter().position(|b| *b == b'\n')
                {
                    Some(i) => (offset + i, offset + i + 1, true),
                    None => (data.len(), data.len(), false),
                };
                let line = &data[offset..end];
                if !line.iter().all(u8::is_ascii_whitespace) {
                    lines.push((offset, next, parse_staged_line(line), terminated));
                }
                offset = next;
            }
            let last_marker = lines
                .iter()
                .rposition(|(_, _, l, _)| matches!(l, StagedLine::Marker(_)));
            if let Some(m) = last_marker {
                for (start, _, line, _) in &lines[..=m] {
                    match line {
                        StagedLine::Bad(e) => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!(
                                    "{}: a line at byte {start} is not a segment or a marker ({e}) and a chunk marker follows it; refusing to cut off committed segments",
                                    path.display()
                                ),
                            ));
                        }
                        StagedLine::Segment(seg) => {
                            segments += 1;
                            committed = committed.max(seg.end_ms);
                        }
                        StagedLine::Marker(n) => committed = committed.max(*n),
                    }
                }
                let (_, next, _, terminated) = &lines[m];
                keep = *next as u64;
                missing_newline = !terminated;
            }
            if keep < data.len() as u64 {
                let f = OpenOptions::new().write(true).open(path)?;
                f.set_len(keep)?;
                f.sync_all()?;
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if missing_newline {
            file.write_all(b"\n")?;
            file.sync_data()?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            file,
            committed_until_ms: committed,
            segments,
            repeats_refused: 0,
            markers: true,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// End of the last committed segment, in ms on the audio clock.
    pub fn committed_until_ms(&self) -> u64 {
        self.committed_until_ms
    }

    pub fn segments(&self) -> u64 {
        self.segments
    }

    pub fn repeats_refused(&self) -> u64 {
        self.repeats_refused
    }

    /// Appends every segment of ONE finished chunk as a unit: one write and one fsync, so a chunk that was
    /// split into halves is committed whole or not at all when the ENGINE dies (its pending segments are
    /// never written). An app crash or power loss in the middle of the one write can still leave a prefix of
    /// the chunk's lines; `open` then puts the cursor at the end of the last complete segment, the chunk is
    /// decoded again and the repeated lines are refused, so nothing is lost, but the join may show a seam
    /// (Phase 4 hardening: write a chunk marker line or a side journal). A segment that starts before the cursor is
    /// refused as a repeat, as in [`Committer::commit`]. Afterwards the cursor is at least `cursor_ms`
    /// (the end of the chunk, also when the chunk gave no segment). Returns, per segment, whether it was
    /// committed.
    pub fn commit_chunk(
        &mut self,
        segs: &[SegmentEvent],
        cursor_ms: u64,
    ) -> std::io::Result<Vec<bool>> {
        let mut buf: Vec<u8> = Vec::new();
        let mut accepted = Vec::with_capacity(segs.len());
        let mut cursor = self.committed_until_ms;
        let mut n = 0u64;
        for seg in segs {
            if seg.start_ms < cursor {
                self.repeats_refused += 1;
                accepted.push(false);
                continue;
            }
            serde_json::to_writer(&mut buf, seg).map_err(std::io::Error::other)?;
            buf.push(b'\n');
            cursor = cursor.max(seg.end_ms);
            n += 1;
            accepted.push(true);
        }
        let new_cursor = cursor.max(cursor_ms);
        // Staged mode closes the chunk with a marker in the same write, also when it gave no segment but moved
        // the cursor.
        if self.markers && (n > 0 || new_cursor > self.committed_until_ms) {
            serde_json::to_writer(&mut buf, &serde_json::json!({ MARKER_KEY: new_cursor }))
                .map_err(std::io::Error::other)?;
            buf.push(b'\n');
        }
        if !buf.is_empty() {
            self.file.write_all(&buf)?;
            self.file.sync_data()?;
        }
        self.committed_until_ms = new_cursor;
        self.segments += n;
        Ok(accepted)
    }

    /// Appends the segment as one line and fsyncs it. Returns `Ok(false)` when it was refused
    /// because it starts before the cursor (a repeat of something committed).
    pub fn commit(&mut self, seg: &SegmentEvent) -> std::io::Result<bool> {
        if seg.start_ms < self.committed_until_ms {
            self.repeats_refused += 1;
            return Ok(false);
        }
        let mut line = serde_json::to_vec(seg).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        self.committed_until_ms = self.committed_until_ms.max(seg.end_ms);
        self.segments += 1;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: u64, end: u64, text: &str) -> SegmentEvent {
        SegmentEvent {
            unit: 0,
            chunk: 0,
            start_ms: start,
            end_ms: end,
            text: text.into(),
            lang: "hi".into(),
            lang_prob: 1.0,
            mixed: false,
            dir: "ltr".into(),
            avg_logprob: -0.3,
            no_speech_prob: 0.01,
            compression_ratio: 1.2,
            model: "m".into(),
            decoded_both_ways: false,
            detection: vec![],
            flags: vec![],
            truncated: false,
            hint_text: None,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wb-supervisor-commit-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d.join("post-1.jsonl")
    }

    #[test]
    fn segments_are_appended_one_per_line_and_move_the_cursor() {
        let p = temp("append");
        let mut c = Committer::open(&p).expect("open");
        assert_eq!(c.committed_until_ms(), 0);
        assert!(c.commit(&seg(0, 4_000, "नमस्ते")).expect("commit"));
        assert!(
            c.commit(&seg(4_000, 9_000, "यह एक test है"))
                .expect("commit")
        );
        assert_eq!((c.committed_until_ms(), c.segments()), (9_000, 2));
        let text = std::fs::read_to_string(&p).expect("read");
        assert_eq!(text.lines().count(), 2);
        assert!(
            text.contains("नमस्ते"),
            "Devanagari is written as it is, not escaped to U+FFFD"
        );
    }

    #[test]
    fn a_segment_that_starts_before_the_cursor_is_refused_and_counted() {
        let p = temp("repeat");
        let mut c = Committer::open(&p).expect("open");
        assert!(c.commit(&seg(0, 4_000, "a")).expect("commit"));
        assert!(!c.commit(&seg(2_000, 5_000, "overlaps")).expect("refused"));
        assert!(!c.commit(&seg(0, 4_000, "a")).expect("refused"));
        assert_eq!(
            (c.segments(), c.repeats_refused(), c.committed_until_ms()),
            (1, 2, 4_000)
        );
        assert_eq!(
            std::fs::read_to_string(&p).expect("read").lines().count(),
            1
        );
    }

    #[test]
    fn reopening_restores_the_cursor_and_the_count() {
        let p = temp("reopen");
        {
            let mut c = Committer::open(&p).expect("open");
            c.commit(&seg(0, 3_000, "a")).expect("commit");
            c.commit(&seg(3_000, 8_000, "b")).expect("commit");
        }
        let mut c = Committer::open(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (8_000, 2));
        assert!(c.commit(&seg(8_000, 12_000, "c")).expect("commit"));
        assert_eq!(
            std::fs::read_to_string(&p).expect("read").lines().count(),
            3
        );
    }

    #[test]
    fn a_torn_last_line_is_cut_off_so_appending_resumes_cleanly() {
        let p = temp("torn");
        {
            let mut c = Committer::open(&p).expect("open");
            c.commit(&seg(0, 3_000, "complete")).expect("commit");
        }
        // A crash in the middle of the next write leaves half a line.
        let mut f = OpenOptions::new().append(true).open(&p).expect("append");
        f.write_all(b"{\"unit\":0,\"chunk\":1,\"start_ms\":3000,\"end_ms\":6")
            .expect("torn write");
        drop(f);
        let mut c = Committer::open(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (3_000, 1));
        assert!(c.commit(&seg(3_000, 6_000, "again")).expect("commit"));
        let text = std::fs::read_to_string(&p).expect("read");
        assert_eq!(text.lines().count(), 2, "{text}");
        for line in text.lines() {
            assert!(serde_json::from_str::<SegmentEvent>(line).is_ok(), "{line}");
        }
    }

    #[test]
    fn a_bad_line_in_the_middle_refuses_the_file_and_cuts_nothing() {
        let p = temp("middle");
        {
            let mut c = Committer::open(&p).expect("open");
            c.commit(&seg(0, 3_000, "a")).expect("commit");
        }
        let mut f = OpenOptions::new().append(true).open(&p).expect("append");
        // A line a newer protocol version wrote (or damage), followed by a good committed line.
        f.write_all(b"{\"future\":true}\n").expect("write");
        let mut good = serde_json::to_vec(&seg(3_000, 6_000, "b")).expect("json");
        good.push(b'\n');
        f.write_all(&good).expect("write");
        drop(f);
        let before = std::fs::read(&p).expect("read");
        let err = match Committer::open(&p) {
            Ok(_) => panic!("a mid-file bad line must refuse the file"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            std::fs::read(&p).expect("read"),
            before,
            "nothing was cut off"
        );
    }

    #[test]
    fn a_last_line_without_its_newline_gets_one_before_the_next_append() {
        let p = temp("no-newline");
        {
            let mut c = Committer::open(&p).expect("open");
            c.commit(&seg(0, 3_000, "a")).expect("commit");
        }
        // Remove the trailing newline: the line still parses.
        let mut bytes = std::fs::read(&p).expect("read");
        assert_eq!(bytes.pop(), Some(b'\n'));
        std::fs::write(&p, &bytes).expect("write");
        let mut c = Committer::open(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (3_000, 1));
        assert!(c.commit(&seg(3_000, 6_000, "b")).expect("commit"));
        let text = std::fs::read_to_string(&p).expect("read");
        assert_eq!(text.lines().count(), 2, "{text}");
        for line in text.lines() {
            assert!(serde_json::from_str::<SegmentEvent>(line).is_ok(), "{line}");
        }
    }

    #[test]
    fn a_torn_last_line_with_a_trailing_blank_is_still_cut() {
        let p = temp("torn-blank");
        {
            let mut c = Committer::open(&p).expect("open");
            c.commit(&seg(0, 3_000, "a")).expect("commit");
        }
        let mut f = OpenOptions::new().append(true).open(&p).expect("append");
        f.write_all(
            b"{\"unit\":0,\"chu

",
        )
        .expect("write");
        drop(f);
        let c = Committer::open(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (3_000, 1));
        assert_eq!(
            std::fs::read_to_string(&p).expect("read").lines().count(),
            1
        );
    }

    // ---- staged mode (chunk-end markers) ----

    #[test]
    fn the_default_mode_writes_no_markers() {
        let p = temp("no-markers");
        let mut c = Committer::open(&p).expect("open");
        c.commit_chunk(&[seg(0, 4_000, "a")], 4_500).expect("chunk");
        let text = std::fs::read_to_string(&p).expect("read");
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains(MARKER_KEY));
    }

    #[test]
    fn a_staged_file_restores_the_cursor_exactly_including_empty_chunks() {
        let p = temp("staged-cursor");
        {
            let mut c = Committer::open_staged(&p).expect("open");
            c.commit_chunk(&[seg(0, 4_000, "a")], 4_500).expect("chunk");
            // A chunk with no segment (too short to decode) still closes with its end.
            c.commit_chunk(&[], 9_000).expect("empty chunk");
            assert_eq!(c.committed_until_ms(), 9_000);
        }
        let c = Committer::open_staged(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (9_000, 1));
    }

    #[test]
    fn a_chunk_cut_short_after_its_first_segment_is_cut_off_whole() {
        let p = temp("staged-partial");
        {
            let mut c = Committer::open_staged(&p).expect("open");
            c.commit_chunk(&[seg(0, 4_000, "a")], 4_000).expect("chunk");
        }
        // A crash in the middle of the next chunk's write: its first segment is complete, its second torn.
        let mut f = OpenOptions::new().append(true).open(&p).expect("append");
        let mut first = serde_json::to_vec(&seg(4_000, 6_000, "b1")).expect("json");
        first.push(b'\n');
        f.write_all(&first).expect("write");
        f.write_all(b"{\"unit\":0,\"chunk\":1,\"start_ms\":6000")
            .expect("torn");
        drop(f);
        let mut c = Committer::open_staged(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (4_000, 1));
        // The chunk is decoded again and committed whole: no seam, no repeat.
        c.commit_chunk(&[seg(4_000, 6_000, "b1"), seg(6_000, 8_000, "b2")], 8_000)
            .expect("again");
        let text = std::fs::read_to_string(&p).expect("read");
        let segs: Vec<_> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<SegmentEvent>(l).ok())
            .collect();
        assert_eq!(
            segs.iter().map(|s| s.start_ms).collect::<Vec<_>>(),
            vec![0, 4_000, 6_000]
        );
        assert_eq!(
            text.lines().count(),
            5,
            "three segments and two markers: {text}"
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_bad_line_above_a_marker_refuses_the_staged_file_and_cuts_nothing() {
        let p = temp("staged-bad");
        {
            let mut c = Committer::open_staged(&p).expect("open");
            c.commit_chunk(&[seg(0, 4_000, "a")], 4_000).expect("chunk");
        }
        let mut f = OpenOptions::new().append(true).open(&p).expect("append");
        f.write_all(b"{\"future\":true}\n{\"chunk_end_ms\":9000}\n")
            .expect("write");
        drop(f);
        let before = std::fs::read(&p).expect("read");
        let err = Committer::open_staged(&p).expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&p).expect("read"), before);
    }

    #[test]
    fn a_staged_file_with_no_marker_yet_starts_empty() {
        let p = temp("staged-nomarker");
        {
            let mut c = Committer::open_staged(&p).expect("open");
            c.commit_chunk(&[seg(0, 4_000, "a")], 4_000).expect("chunk");
        }
        // Only the segment line of a first chunk survived: no marker, so nothing is committed.
        let text = std::fs::read_to_string(&p).expect("read");
        let first = text.lines().next().expect("segment line");
        std::fs::write(&p, format!("{first}\n")).expect("write");
        let c = Committer::open_staged(&p).expect("reopen");
        assert_eq!((c.committed_until_ms(), c.segments()), (0, 0));
        assert_eq!(std::fs::metadata(&p).expect("meta").len(), 0);
    }
}
