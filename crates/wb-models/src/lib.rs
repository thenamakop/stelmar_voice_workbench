//! The model manifest and checksum verification.
//!
//! `models/manifest.json` (committed, metadata only: ids, file names, sizes, SHA-256 values,
//! licences, sources and how a file was converted) says which model files exist. The model files
//! themselves are never committed and never downloaded by the app. At start-up the app hashes each
//! file in the background; a missing, truncated or altered file is marked and disabled, and no
//! engine is started with it (AGENTS.md; PLAN section 10).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where a model came from, exactly enough to fetch the same bytes again.
///
/// Written as `{"repo": ..., "revision": ...}` in `models/manifest.json`. The development-time
/// `manifest.local.json` (written by `scripts/models/*.py`) keeps one string, `https://huggingface.co/<repo>@<revision>`,
/// optionally followed by a note in brackets; that is read as the same two fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SourceRepr")]
pub struct Source {
    /// For example `ggerganov/whisper.cpp` or `vasista22/whisper-hindi-medium`.
    #[serde(default)]
    pub repo: String,
    /// The exact commit or revision of that repository.
    #[serde(default)]
    pub revision: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SourceRepr {
    Text(String),
    Fields {
        #[serde(default)]
        repo: String,
        #[serde(default)]
        revision: String,
    },
}

impl From<SourceRepr> for Source {
    fn from(r: SourceRepr) -> Self {
        match r {
            SourceRepr::Fields { repo, revision } => Source { repo, revision },
            SourceRepr::Text(t) => {
                let head = t.split([' ', '(']).next().unwrap_or("");
                let head = head.strip_prefix("https://huggingface.co/").unwrap_or(head);
                match head.split_once('@') {
                    Some((repo, rev)) => Source {
                        repo: repo.to_string(),
                        revision: rev.to_string(),
                    },
                    None => Source {
                        repo: head.to_string(),
                        revision: String::new(),
                    },
                }
            }
        }
    }
}

/// How a converted or quantised file was made from its input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversion {
    /// The script that made it (`scripts/models/convert_hf_whisper.py`, `whisper-quantize`).
    #[serde(default)]
    pub script: String,
    /// Settings that change the output: quantisation type, input precision, tokenizer handling.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// SHA-256 of the file it was made from.
    #[serde(default)]
    pub input_sha256: String,
    /// The whisper.cpp commit whose tools made it.
    #[serde(default)]
    pub whisper_cpp_commit: String,
    /// Whether making it again from the same input gives the same SHA-256 (measured, not assumed).
    #[serde(default)]
    pub deterministic: Option<bool>,
}

/// A model's place in the ranking of one role and language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankPlace {
    /// `rolling`, `night`, `near_live` or `live`.
    pub role: String,
    /// `en`, `hi` or `ur`.
    pub lang: String,
    /// 1 is tried first.
    pub rank: u32,
    /// No local, independent WER backs this place.
    #[serde(default)]
    pub quality_unverified: bool,
}

/// One model in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// File name relative to the models folder.
    pub file: String,
    /// Lower-case hex SHA-256 of the file; a placeholder (`<fill in ...>`) means "not yet filled".
    pub sha256: String,
    pub bytes: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub quant: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub candidate_roles: Vec<String>,
    /// Roles the model passed in evaluation; the system check uses a model only for these.
    #[serde(default)]
    pub validated_roles: Vec<String>,
    /// Places this model holds in the system check's ranking, per role and language (1 = first tried).
    /// Provisional until Phase 5 validates models; the check measures them top-down.
    #[serde(default)]
    pub provisional_rank: Vec<RankPlace>,
    #[serde(default)]
    pub licence: String,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub conversion: Option<Conversion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub models: Vec<Entry>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: not a valid model manifest: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("model {0} is not in the manifest")]
    Unknown(String),
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn find(&self, id: &str) -> Result<&Entry, Error> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::Unknown(id.to_string()))
    }
}

/// True for the template's `<fill in after download>` markers and anything that is not 64 hex digits.
pub fn is_placeholder(sha256: &str) -> bool {
    !(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// What the check of one model found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// The file is there and its SHA-256 equals the manifest's.
    Ok,
    Missing,
    /// The file is a different size than the manifest says (truncated or replaced).
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    /// The same size but other bytes.
    HashMismatch {
        expected: String,
        actual: String,
    },
    /// The manifest has no real SHA-256 for this entry yet, so nothing can be checked.
    NoChecksum,
}

impl Status {
    /// An engine may be started with the model only in this state.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// Short text for a badge or a Toast.
    pub fn describe(&self) -> String {
        match self {
            Self::Ok => "checksum matches".into(),
            Self::Missing => "file not found".into(),
            Self::SizeMismatch { expected, actual } => {
                format!("wrong size ({actual} bytes, expected {expected})")
            }
            Self::HashMismatch { .. } => "checksum does not match".into(),
            Self::NoChecksum => "no checksum in the manifest".into(),
        }
    }
}

/// SHA-256 of a file, read in 1 MiB blocks.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A file that was hashed once: it need not be again while its size and modification time stay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    pub size: u64,
    pub mtime_ns: u128,
    pub sha256: String,
}

/// Hashes already done, kept between runs (`verified.json` in the app data folder) so start-up does
/// not read gigabytes again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub files: BTreeMap<String, Verified>,
}

impl Cache {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        fs::rename(&tmp, path)
    }
}

/// A file modified less than this long before it was hashed is not cached. File systems stamp
/// modification times coarsely (a clock tick or more), so a same-size edit made just after the hash
/// would keep the same size and time and be taken for the verified file. Model files are hours old
/// when they are verified, so in practice nothing is re-read that need not be.
const RACY_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

fn is_racy(meta: &fs::Metadata) -> bool {
    match meta.modified().map(|t| t.elapsed()) {
        Ok(Ok(age)) => age < RACY_WINDOW,
        // A modification time in the future, or unreadable: do not trust it.
        _ => true,
    }
}

fn mtime_ns(meta: &fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Checks one model file against its manifest entry. Reads the whole file unless `cache` already
/// holds its hash for the same size and modification time. `hashed` is set when the file was read.
pub fn verify(dir: &Path, entry: &Entry, cache: &mut Cache, hashed: &mut bool) -> Status {
    *hashed = false;
    let path = dir.join(&entry.file);
    let Ok(meta) = fs::metadata(&path) else {
        return Status::Missing;
    };
    if !meta.is_file() {
        return Status::Missing;
    }
    if is_placeholder(&entry.sha256) {
        return Status::NoChecksum;
    }
    if entry.bytes != 0 && meta.len() != entry.bytes {
        return Status::SizeMismatch {
            expected: entry.bytes,
            actual: meta.len(),
        };
    }
    let mtime = mtime_ns(&meta);
    let want = entry.sha256.to_ascii_lowercase();
    if let Some(v) = cache.files.get(&entry.file)
        && v.size == meta.len()
        && v.mtime_ns == mtime
        && v.sha256 == want
    {
        return Status::Ok;
    }
    *hashed = true;
    match sha256_file(&path) {
        Ok(actual) if actual == want => {
            if is_racy(&meta) {
                // Verified now, but not remembered: the next check reads the file again.
                cache.files.remove(&entry.file);
            } else {
                cache.files.insert(
                    entry.file.clone(),
                    Verified {
                        size: meta.len(),
                        mtime_ns: mtime,
                        sha256: actual,
                    },
                );
            }
            Status::Ok
        }
        Ok(actual) => {
            cache.files.remove(&entry.file);
            Status::HashMismatch {
                expected: want,
                actual,
            }
        }
        Err(_) => Status::Missing,
    }
}

/// The path of a model the app may hand to an engine. Re-checks the file when its size or
/// modification time changed since it was verified, and refuses any model that is not `Ok`.
pub fn path_for_engine(
    dir: &Path,
    manifest: &Manifest,
    id: &str,
    cache: &mut Cache,
) -> Result<PathBuf, String> {
    let entry = manifest.find(id).map_err(|e| e.to_string())?;
    let mut hashed = false;
    let status = verify(dir, entry, cache, &mut hashed);
    if status.is_ok() {
        Ok(dir.join(&entry.file))
    } else {
        Err(format!("model {id} cannot be used: {}", status.describe()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let d = std::env::temp_dir().join(format!(
                "wb-models-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).expect("temp dir");
            Self(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn entry(file: &str, bytes: &[u8]) -> Entry {
        Entry {
            id: "m".into(),
            file: file.into(),
            sha256: sha(bytes),
            bytes: bytes.len() as u64,
            kind: "whisper".into(),
            base: String::new(),
            quant: String::new(),
            languages: vec![],
            candidate_roles: vec![],
            validated_roles: vec![],
            provisional_rank: vec![],
            licence: "MIT".into(),
            source: Source::default(),
            conversion: None,
        }
    }

    /// Makes a file look old (as a model file is when it is verified), so it may be cached.
    fn age(dir: &Dir, file: &str) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(dir.0.join(file))
            .and_then(|f| f.set_modified(old))
            .expect("set modification time");
    }

    fn check(dir: &Dir, e: &Entry, cache: &mut Cache) -> (Status, bool) {
        let mut hashed = false;
        let s = verify(&dir.0, e, cache, &mut hashed);
        (s, hashed)
    }

    #[test]
    fn an_intact_file_is_ok_and_is_not_read_again_while_it_is_unchanged() {
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"model bytes").expect("write");
        age(&d, "a.bin");
        let e = entry("a.bin", b"model bytes");
        let mut cache = Cache::default();
        assert_eq!(check(&d, &e, &mut cache), (Status::Ok, true));
        assert_eq!(
            check(&d, &e, &mut cache),
            (Status::Ok, false),
            "second check uses the cache"
        );
    }

    #[test]
    fn a_file_modified_just_before_it_was_hashed_is_not_cached() {
        // Coarse file-system timestamps: a same-size edit right after the hash would look unchanged.
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"model bytes").expect("write");
        let e = entry("a.bin", b"model bytes");
        let mut cache = Cache::default();
        assert_eq!(check(&d, &e, &mut cache), (Status::Ok, true));
        assert!(
            cache.files.is_empty(),
            "a racy file is verified, not remembered"
        );
        assert_eq!(
            check(&d, &e, &mut cache),
            (Status::Ok, true),
            "so the next check reads it again"
        );
    }

    #[test]
    fn one_corrupted_byte_is_a_hash_mismatch() {
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"model bytes").expect("write");
        age(&d, "a.bin");
        let e = entry("a.bin", b"model bytes");
        let mut cache = Cache::default();
        assert!(check(&d, &e, &mut cache).0.is_ok());
        assert!(
            cache.files.contains_key("a.bin"),
            "an old file is remembered"
        );
        fs::write(d.0.join("a.bin"), b"model byteX").expect("write");
        let (status, hashed) = check(&d, &e, &mut cache);
        assert!(hashed, "a changed file is read again");
        assert!(matches!(status, Status::HashMismatch { .. }), "{status:?}");
        assert!(!status.is_ok());
        assert!(
            !cache.files.contains_key("a.bin"),
            "a failed check leaves nothing in the cache"
        );
    }

    #[test]
    fn a_truncated_file_is_a_size_mismatch_without_hashing_it() {
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"model").expect("write");
        let e = entry("a.bin", b"model bytes");
        let mut cache = Cache::default();
        let (status, hashed) = check(&d, &e, &mut cache);
        assert_eq!(
            status,
            Status::SizeMismatch {
                expected: 11,
                actual: 5
            }
        );
        assert!(!hashed);
    }

    #[test]
    fn a_missing_file_or_a_folder_is_missing() {
        let d = Dir::new();
        let e = entry("nope.bin", b"x");
        let mut cache = Cache::default();
        assert_eq!(check(&d, &e, &mut cache).0, Status::Missing);
        fs::create_dir_all(d.0.join("dir.bin")).expect("dir");
        assert_eq!(
            check(&d, &entry("dir.bin", b"x"), &mut cache).0,
            Status::Missing
        );
    }

    #[test]
    fn a_placeholder_checksum_cannot_be_checked_and_is_not_usable() {
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"x").expect("write");
        let mut e = entry("a.bin", b"x");
        e.sha256 = "<fill in after download>".into();
        assert!(is_placeholder(&e.sha256));
        assert!(!is_placeholder(&sha(b"x")));
        let mut cache = Cache::default();
        let (status, _) = check(&d, &e, &mut cache);
        assert_eq!(status, Status::NoChecksum);
        assert!(!status.is_ok());
    }

    #[test]
    fn the_engine_only_gets_a_path_for_a_verified_model() {
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"model bytes").expect("write");
        let manifest = Manifest {
            version: 2,
            models: vec![entry("a.bin", b"model bytes")],
        };
        let mut cache = Cache::default();
        assert_eq!(
            path_for_engine(&d.0, &manifest, "m", &mut cache),
            Ok(d.0.join("a.bin"))
        );
        // Altered after it was verified (same size): refused on the next start.
        fs::write(d.0.join("a.bin"), b"model byteZ").expect("write");
        let err = path_for_engine(&d.0, &manifest, "m", &mut cache).unwrap_err();
        assert!(err.contains("checksum does not match"), "{err}");
        assert!(
            path_for_engine(&d.0, &manifest, "unknown", &mut cache)
                .unwrap_err()
                .contains("not in the manifest")
        );
    }

    #[test]
    fn the_cache_survives_a_restart() {
        let d = Dir::new();
        fs::write(d.0.join("a.bin"), b"model bytes").expect("write");
        age(&d, "a.bin");
        let e = entry("a.bin", b"model bytes");
        let mut cache = Cache::default();
        assert!(check(&d, &e, &mut cache).0.is_ok());
        let cache_file = d.0.join("verified.json");
        cache.save(&cache_file).expect("save");
        let mut reloaded = Cache::load(&cache_file);
        assert_eq!(check(&d, &e, &mut reloaded), (Status::Ok, false));
        // A damaged cache file is ignored, not fatal.
        fs::write(&cache_file, b"not json").expect("write");
        assert_eq!(Cache::load(&cache_file), Cache::default());
    }

    #[test]
    fn the_manifest_parses_with_and_without_the_optional_fields() {
        let text = r#"{"version":2,"models":[
            {"id":"small","file":"ggml-small.bin","sha256":"<fill in after download>","bytes":0},
            {"id":"hindi-small-q8_0","file":"x.bin","sha256":"ab","bytes":5,"languages":["hi"],
             "source":{"repo":"vasista22/whisper-hindi-small","revision":"abc"},
             "conversion":{"script":"scripts/models/convert_hf_whisper.py","settings":{"quant":"q8_0"},"deterministic":true}}]}"#;
        let m: Manifest = serde_json::from_str(text).expect("parse");
        assert_eq!(m.find("small").expect("small").bytes, 0);
        let c = m
            .find("hindi-small-q8_0")
            .expect("hindi")
            .conversion
            .clone()
            .expect("conversion");
        assert_eq!(c.deterministic, Some(true));
        assert_eq!(c.settings.get("quant").map(String::as_str), Some("q8_0"));
        assert!(m.find("missing").is_err());
    }

    #[test]
    fn source_reads_both_the_object_and_the_local_string_form() {
        let a: Source =
            serde_json::from_str(r#"{"repo":"ggerganov/whisper.cpp","revision":"5359861c"}"#)
                .expect("object");
        let b: Source =
            serde_json::from_str(r#""https://huggingface.co/ggerganov/whisper.cpp@5359861c""#)
                .expect("string");
        let c: Source = serde_json::from_str(r#""https://huggingface.co/vasista22/whisper-hindi-small@abc (convert-h5-to-ggml.py, then quantize)""#).expect("with note");
        assert_eq!(a, b);
        assert_eq!(
            (c.repo.as_str(), c.revision.as_str()),
            ("vasista22/whisper-hindi-small", "abc")
        );
    }
}
