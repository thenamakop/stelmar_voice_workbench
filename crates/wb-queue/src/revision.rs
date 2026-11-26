//! The single writer of a session's transcript files (AGENTS.md "Single writer").
//!
//! A finished unit's segments are appended to the revision's JSONL (`post-1.jsonl`, `post-2.jsonl`) in one
//! write followed by `sync_data`; only then are they inserted into SQLite in one transaction, and only then
//! may the app show them. The JSONL is the truth: [`Transcript::open`] cuts a torn last line and inserts any
//! segment SQLite is missing, so a crash at any point leaves nothing lost and nothing doubled.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::store::{Segment, Store};
use crate::{Error, Result, io};

/// What one [`Transcript::commit`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Committed {
    /// Segments written.
    pub written: usize,
    /// Segments refused because they start before the revision's committed end (a repeat).
    pub refused: usize,
}

struct RevisionFile {
    path: PathBuf,
    file: File,
    /// End of the last committed segment; nothing may start before it.
    committed_end_ms: u64,
}

/// A session's transcript: its `transcript.db` plus one JSONL per revision.
pub struct Transcript {
    dir: PathBuf,
    store: Store,
    revisions: HashMap<String, RevisionFile>,
    /// A commit that fails on purpose (tests); never set in a real run.
    fail_after: Option<u32>,
    /// Segments already written to the JSONL (the truth) that SQLite does not have yet because its insert failed. The next
    /// commit inserts them before anything else, and a repeat of one of them is not refused (N3 of the auditor of 20 Nov 2026).
    unindexed: Vec<Segment>,
    /// The next SQLite insert fails once (tests).
    fail_index_once: bool,
}

pub fn jsonl_path(dir: &Path, revision: &str) -> PathBuf {
    dir.join(format!("{revision}.jsonl"))
}

/// Reads a revision's JSONL, cutting a torn last line (and fsyncing the cut). A bad line followed by more
/// lines is corruption, not a torn write, and is an error: nothing is cut.
fn load_jsonl(path: &Path) -> Result<Vec<Segment>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(path)(e)),
    };
    let mut segments = Vec::new();
    let mut good_end = 0usize;
    let mut pos = 0usize;
    while pos < bytes.len() {
        let (line_end, next, terminated) = match bytes[pos..].iter().position(|b| *b == b'\n') {
            Some(i) => (pos + i, pos + i + 1, true),
            None => (bytes.len(), bytes.len(), false),
        };
        let line = &bytes[pos..line_end];
        if line.iter().all(u8::is_ascii_whitespace) {
            good_end = next;
            pos = next;
            continue;
        }
        match serde_json::from_slice::<Segment>(line) {
            // A last line that parses but has no newline is complete: keep it (the newline is added below).
            Ok(seg) => {
                segments.push(seg);
                good_end = next;
            }
            Err(e) => {
                if next < bytes.len() || terminated && next < bytes.len() {
                    return Err(Error::Invalid(format!(
                        "{}: a bad line in the middle of the file ({e}); nothing was cut",
                        path.display()
                    )));
                }
                // A torn last line: cut it.
                let f = OpenOptions::new()
                    .write(true)
                    .open(path)
                    .map_err(io(path))?;
                f.set_len(good_end as u64).map_err(io(path))?;
                f.sync_all().map_err(io(path))?;
                return Ok(segments);
            }
        }
        pos = next;
    }
    if good_end != bytes.len() || bytes.last().is_some_and(|b| *b != b'\n') {
        let mut f = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(io(path))?;
        f.write_all(b"\n").map_err(io(path))?;
        f.sync_all().map_err(io(path))?;
    }
    Ok(segments)
}

impl Transcript {
    /// Opens the session's transcript, recovering from a crash: every revision JSONL in the folder is read
    /// (a torn last line is cut) and any segment SQLite does not have is inserted.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        let mut store = Store::open(&dir.join("transcript.db"))?;
        let mut revisions = HashMap::new();
        for entry in std::fs::read_dir(dir).map_err(io(dir))? {
            let entry = entry.map_err(io(dir))?;
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
                .filter(|n| n.starts_with("post-"))
                .map(str::to_owned)
            else {
                continue;
            };
            let segments = load_jsonl(&path)?;
            store.ensure_revision(&name)?;
            store.insert_segments(&segments)?;
            let committed_end_ms = segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
            let file = OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(io(&path))?;
            revisions.insert(
                name,
                RevisionFile {
                    path,
                    file,
                    committed_end_ms,
                },
            );
        }
        Ok(Transcript {
            dir: dir.to_owned(),
            store,
            revisions,
            fail_after: None,
            unindexed: Vec::new(),
            fail_index_once: false,
        })
    }

    /// The commit after `ok_commits` successful ones fails once, before anything is written (tests of the callers'
    /// error paths).
    #[doc(hidden)]
    pub fn inject_commit_failure_after(&mut self, ok_commits: u32) {
        self.fail_after = Some(ok_commits);
    }

    /// The next SQLite insert of a commit fails once, after the JSONL was written (tests of the retry).
    #[doc(hidden)]
    pub fn inject_index_failure_once(&mut self) {
        self.fail_index_once = true;
    }

    /// Inserts into SQLite the segments the JSONL has and SQLite lacks; returns how many were added.
    fn index_pending(&mut self) -> Result<usize> {
        if self.unindexed.is_empty() {
            return Ok(0);
        }
        if std::mem::take(&mut self.fail_index_once) {
            return Err(Error::Invalid("index failure injected by a test".into()));
        }
        self.store.insert_segments(&self.unindexed)?;
        let n = self.unindexed.len();
        self.unindexed.clear();
        Ok(n)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// End of the last committed segment of a revision (0 if none).
    pub fn committed_end_ms(&self, revision: &str) -> u64 {
        self.revisions
            .get(revision)
            .map_or(0, |r| r.committed_end_ms)
    }

    fn revision_file(&mut self, revision: &str) -> Result<&mut RevisionFile> {
        if !self.revisions.contains_key(revision) {
            let path = jsonl_path(&self.dir, revision);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(io(&path))?;
            self.store.ensure_revision(revision)?;
            self.revisions.insert(
                revision.to_owned(),
                RevisionFile {
                    path,
                    file,
                    committed_end_ms: 0,
                },
            );
        }
        self.revisions
            .get_mut(revision)
            .ok_or_else(|| Error::Invalid("revision file missing".into()))
    }

    /// Commits one unit's segments (already in audio order) to `revision`: refuses any segment that starts
    /// before the revision's committed end, writes the rest with one `write_all` and one `sync_data`, then
    /// inserts them into SQLite. Returns what was written and what was refused as a repeat.
    pub fn commit(&mut self, revision: &str, segments: &[Segment]) -> Result<Committed> {
        match self.fail_after {
            Some(0) => {
                self.fail_after = None;
                return Err(Error::Invalid("commit failure injected by a test".into()));
            }
            Some(n) => self.fail_after = Some(n - 1),
            None => {}
        }
        // A commit whose SQLite insert failed left its segments in the JSONL only: put them into SQLite first. A retry that
        // offers the same segments again is then not a repeat to refuse: they count as written (they became visible now).
        let pending: Vec<(u64, u64, String)> = self
            .unindexed
            .iter()
            .filter(|u| u.revision == revision)
            .map(|u| (u.start_ms, u.end_ms, u.text.clone()))
            .collect();
        let mut indexed_now = self.index_pending()?;
        let mut accepted: Vec<&Segment> = Vec::new();
        let mut refused = 0;
        let mut retried = 0;
        {
            let file = self.revision_file(revision)?;
            let mut end = file.committed_end_ms;
            for s in segments {
                if pending.iter().any(|(a, b, t)| {
                    (*a, *b, t.as_str()) == (s.start_ms, s.end_ms, s.text.as_str())
                }) {
                    retried += 1;
                } else if s.start_ms < end || s.end_ms < s.start_ms {
                    refused += 1;
                } else {
                    end = s.end_ms.max(s.start_ms);
                    accepted.push(s);
                }
            }
        }
        if accepted.is_empty() {
            indexed_now = indexed_now.min(retried);
            return Ok(Committed {
                written: indexed_now,
                refused,
            });
        }
        let mut buf = Vec::new();
        for s in &accepted {
            if s.revision != revision {
                return Err(Error::Invalid(format!(
                    "segment of revision {} given to {revision}",
                    s.revision
                )));
            }
            serde_json::to_writer(&mut buf, s)?;
            buf.push(b'\n');
        }
        let new_end = accepted.iter().map(|s| s.end_ms).max().unwrap_or(0);
        {
            let file = self.revision_file(revision)?;
            let before = file.file.metadata().map_err(io(&file.path))?.len();
            if let Err(e) = file
                .file
                .write_all(&buf)
                .and_then(|()| file.file.sync_data())
            {
                // A torn line would make the whole file unreadable and the next append would land behind it: cut the
                // file back to the whole lines it held before (best effort; the append handle cannot truncate).
                let _ = OpenOptions::new()
                    .write(true)
                    .open(&file.path)
                    .and_then(|f| f.set_len(before));
                return Err(e).map_err(io(&file.path));
            }
            file.committed_end_ms = new_end;
        }
        let owned: Vec<Segment> = accepted.into_iter().cloned().collect();
        let written = owned.len() + indexed_now.min(retried);
        self.unindexed.extend(owned);
        self.index_pending()?;
        Ok(Committed { written, refused })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{seg, temp};

    /// N3 (auditor of 20 Nov 2026): the JSONL write succeeded and the SQLite insert failed. The retry of the same segments
    /// used to be refused as repeats (the committed end had moved) and SQLite lacked them until the next open.
    #[test]
    fn a_retry_after_a_failed_sqlite_insert_puts_the_segments_into_sqlite_and_refuses_nothing() {
        let dir = temp("rev-index-retry");
        let mut t = Transcript::open(&dir).unwrap();
        let segs = [seg("post-1", 0, 900, "a"), seg("post-1", 1_000, 1_900, "b")];
        t.inject_index_failure_once();
        assert!(t.commit("post-1", &segs).is_err());
        assert_eq!(t.committed_end_ms("post-1"), 1_900, "the JSONL has them");
        assert_eq!(
            t.store().segment_count("post-1").unwrap(),
            0,
            "SQLite does not"
        );
        let c = t.commit("post-1", &segs).unwrap();
        assert_eq!((c.written, c.refused), (2, 0));
        assert_eq!(t.store().segment_count("post-1").unwrap(), 2);
        // A genuine repeat afterwards is refused as before, and the next unit commits normally.
        let again = t.commit("post-1", &segs).unwrap();
        assert_eq!((again.written, again.refused), (0, 2));
        let next = t
            .commit("post-1", &[seg("post-1", 2_000, 2_500, "c")])
            .unwrap();
        assert_eq!((next.written, next.refused), (1, 0));
        assert_eq!(t.store().segment_count("post-1").unwrap(), 3);
    }

    #[test]
    fn a_commit_is_in_the_jsonl_and_in_sqlite() {
        let dir = temp("rev-basic");
        let mut t = Transcript::open(&dir).unwrap();
        let c = t
            .commit(
                "post-1",
                &[seg("post-1", 0, 900, "a"), seg("post-1", 1_000, 1_900, "b")],
            )
            .unwrap();
        assert_eq!(
            c,
            Committed {
                written: 2,
                refused: 0
            }
        );
        assert_eq!(t.store().segment_count("post-1").unwrap(), 2);
        let text = std::fs::read_to_string(jsonl_path(&dir, "post-1")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(t.committed_end_ms("post-1"), 1_900);
    }

    #[test]
    fn a_repeat_is_refused_and_the_files_do_not_change() {
        let dir = temp("rev-repeat");
        let mut t = Transcript::open(&dir).unwrap();
        t.commit(
            "post-1",
            &[seg("post-1", 0, 900, "a"), seg("post-1", 1_000, 1_900, "b")],
        )
        .unwrap();
        let before = std::fs::read(jsonl_path(&dir, "post-1")).unwrap();
        let c = t
            .commit(
                "post-1",
                &[
                    seg("post-1", 1_000, 1_900, "b"),
                    seg("post-1", 500, 800, "x"),
                ],
            )
            .unwrap();
        assert_eq!(
            c,
            Committed {
                written: 0,
                refused: 2
            }
        );
        assert_eq!(std::fs::read(jsonl_path(&dir, "post-1")).unwrap(), before);
    }

    #[test]
    fn a_crash_after_the_jsonl_write_is_repaired_from_the_jsonl() {
        let dir = temp("rev-crash");
        {
            let mut t = Transcript::open(&dir).unwrap();
            t.commit("post-1", &[seg("post-1", 0, 900, "a")]).unwrap();
        }
        // Simulate: the JSONL got a second segment (fsynced) but the process died before SQLite did.
        let line = serde_json::to_string(&seg("post-1", 1_000, 1_900, "b")).unwrap();
        let mut f = OpenOptions::new()
            .append(true)
            .open(jsonl_path(&dir, "post-1"))
            .unwrap();
        writeln!(f, "{line}").unwrap();
        drop(f);
        let t = Transcript::open(&dir).unwrap();
        assert_eq!(t.store().segment_count("post-1").unwrap(), 2);
        assert_eq!(t.committed_end_ms("post-1"), 1_900);
    }

    #[test]
    fn a_torn_last_line_is_cut_and_the_rest_is_kept() {
        let dir = temp("rev-torn");
        {
            let mut t = Transcript::open(&dir).unwrap();
            t.commit("post-1", &[seg("post-1", 0, 900, "a")]).unwrap();
        }
        let line = serde_json::to_string(&seg("post-1", 1_000, 1_900, "b")).unwrap();
        let mut f = OpenOptions::new()
            .append(true)
            .open(jsonl_path(&dir, "post-1"))
            .unwrap();
        f.write_all(&line.as_bytes()[..line.len() / 2]).unwrap();
        drop(f);
        let mut t = Transcript::open(&dir).unwrap();
        assert_eq!(t.store().segment_count("post-1").unwrap(), 1);
        assert_eq!(t.committed_end_ms("post-1"), 900);
        // The cut segment can be committed again without a seam.
        t.commit("post-1", &[seg("post-1", 1_000, 1_900, "b")])
            .unwrap();
        let text = std::fs::read_to_string(jsonl_path(&dir, "post-1")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_last_line_that_parses_but_lacks_its_newline_is_kept() {
        let dir = temp("rev-nonl");
        let line = serde_json::to_string(&seg("post-1", 0, 900, "a")).unwrap();
        std::fs::write(jsonl_path(&dir, "post-1"), line).unwrap();
        let t = Transcript::open(&dir).unwrap();
        assert_eq!(t.store().segment_count("post-1").unwrap(), 1);
        assert!(
            std::fs::read_to_string(jsonl_path(&dir, "post-1"))
                .unwrap()
                .ends_with('\n')
        );
    }

    #[test]
    fn a_bad_line_in_the_middle_is_an_error_and_nothing_is_cut() {
        let dir = temp("rev-bad");
        let good = serde_json::to_string(&seg("post-1", 0, 900, "a")).unwrap();
        let content = format!("{good}\nnot json\n{good}\n");
        std::fs::write(jsonl_path(&dir, "post-1"), &content).unwrap();
        assert!(Transcript::open(&dir).is_err());
        assert_eq!(
            std::fs::read_to_string(jsonl_path(&dir, "post-1")).unwrap(),
            content
        );
    }

    #[test]
    fn revisions_are_separate_files() {
        let dir = temp("rev-two");
        let mut t = Transcript::open(&dir).unwrap();
        t.commit("post-1", &[seg("post-1", 0, 900, "a")]).unwrap();
        t.commit("post-2", &[seg("post-2", 0, 900, "A")]).unwrap();
        assert_eq!(t.store().segment_count("post-1").unwrap(), 1);
        assert_eq!(t.store().segment_count("post-2").unwrap(), 1);
        assert!(jsonl_path(&dir, "post-2").is_file());
        assert!(
            t.commit("post-1", &[seg("post-2", 2_000, 2_100, "x")])
                .is_err()
        );
    }
}
