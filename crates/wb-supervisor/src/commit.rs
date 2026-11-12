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

#[derive(Debug)]
pub struct Committer {
    path: PathBuf,
    file: File,
    committed_until_ms: u64,
    segments: u64,
    repeats_refused: u64,
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
        if !buf.is_empty() {
            self.file.write_all(&buf)?;
            self.file.sync_data()?;
        }
        self.committed_until_ms = cursor.max(cursor_ms);
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
}
