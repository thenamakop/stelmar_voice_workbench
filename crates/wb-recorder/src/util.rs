//! Small helpers: atomic file writes, IST timestamps, SHA-256 of files, folder-name slugs.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// India Standard Time. Wall-clock times are stored and shown in IST (AGENTS.md, Time).
pub const IST: UtcOffset = time::macros::offset!(+5:30);

/// Now, in IST.
pub fn now_ist() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(IST)
}

/// RFC 3339 with the +05:30 offset, for example `2027-02-08T12:00:03.25+05:30`.
pub fn rfc3339(dt: OffsetDateTime) -> String {
    dt.to_offset(IST)
        .format(&Rfc3339)
        .unwrap_or_else(|_| dt.unix_timestamp().to_string())
}

/// Parses an RFC 3339 timestamp written by `rfc3339`.
pub fn parse_rfc3339(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

/// The IST calendar date, `YYYY-MM-DD`.
pub fn ist_date(dt: OffsetDateTime) -> String {
    let d = dt.to_offset(IST).date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// Writes `bytes` to `path` so a reader sees either the old or the new file, never half of one:
/// write a temporary file beside it, fsync, then rename over the target. Retries briefly when
/// another process (an antivirus scanner, the mirror) holds the target open.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let mut last_err = None;
    for attempt in 0..10u64 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(20 * (attempt + 1)));
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(last_err.unwrap_or_else(|| io::Error::other("rename failed")))
}

/// Serialises `value` as pretty JSON and writes it atomically.
pub fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

/// Reads and parses a JSON file.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Lower-case hex SHA-256 of a file's contents.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = OpenOptions::new().read(true).open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Lower-case hex of a byte slice.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

/// A folder-name slug: ASCII letters and digits, runs of anything else become one hyphen, at most
/// `max` characters. Titles in Devanagari or Nastaliq give an empty slug; callers supply a
/// fallback.
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.truncate(max);
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Path of `path` relative to `base`, with forward slashes (for JSON and manifests).
pub fn rel_slash(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

/// Every regular file under `dir`, recursively, sorted.
pub fn files_under(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_keep_ascii_words() {
        assert_eq!(slug("Hall A", 60), "hall-a");
        assert_eq!(
            slug("Inaugural Session: Inaugural Keynote!", 60),
            "inaugural-session-inaugural-keynote"
        );
        assert_eq!(slug("  --Parallel   Track I-- ", 60), "parallel-track-i");
        assert_eq!(slug("उद्घाटन सत्र", 60), "");
        assert_eq!(slug("abc def ghi", 5), "abc-d");
        assert_eq!(slug("abcd efgh", 5), "abcd");
    }

    #[test]
    fn ist_formatting() -> Result<(), Box<dyn std::error::Error>> {
        // 18:40 UTC on 7 February is 00:10 IST on 8 February.
        let dt = time::macros::datetime!(2027-02-07 18:40:00 UTC);
        assert_eq!(ist_date(dt), "2027-02-08");
        assert_eq!(
            ist_date(time::macros::datetime!(2027-02-07 18:29:59 UTC)),
            "2027-02-07"
        );
        let s = rfc3339(dt);
        assert!(s.ends_with("+05:30"), "{s}");
        assert_eq!(parse_rfc3339(&s), Some(dt));
        Ok(())
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
    }
}
