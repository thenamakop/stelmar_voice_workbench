//! Pins what the engine is built from.
//!
//! Computes the SHA-256 of the vendored whisper.cpp tree (`third_party/whisper.cpp`), compares it
//! with the hash pinned in `third_party/README.md` and stops the build when they differ, so an
//! engine can never be built from a source tree other than the reviewed one. The hash, the commit
//! and the tag are compiled in; `wb-engine info`, `bench` and `batch` print them, and the evidence
//! of every run shows what produced it.
//!
//! The hash covers, for every file under the tree in ordinal byte order of its relative path
//! (separator "/"): the path bytes, one 0x00 byte, the length as 8 bytes little-endian, and the
//! file bytes. `scripts/check-vendor.mjs` computes the same value.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, root, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    Ok(())
}

fn tree_hash(root: &Path) -> std::io::Result<(String, usize)> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut h = Sha256::new();
    let mut buf = Vec::new();
    for rel in &files {
        buf.clear();
        let mut f = fs::File::open(root.join(rel.split('/').collect::<PathBuf>()))?;
        f.read_to_end(&mut buf)?;
        h.update(rel.as_bytes());
        h.update([0u8]);
        h.update((buf.len() as u64).to_le_bytes());
        h.update(&buf);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok((hex, files.len()))
}

/// The value after `key:` on its own line of `third_party/README.md`.
fn pinned(readme: &str, key: &str) -> Option<String> {
    readme.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix(key)?.strip_prefix(':')?;
        Some(rest.trim().trim_matches('`').to_string())
    })
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let third_party = manifest.join("..").join("..").join("third_party");
    let tree = third_party.join("whisper.cpp");
    let readme_path = third_party.join("README.md");
    println!("cargo:rerun-if-changed={}", tree.display());
    println!("cargo:rerun-if-changed={}", readme_path.display());
    println!("cargo:rerun-if-env-changed=WB_ENGINE_VARIANT");

    let readme =
        fs::read_to_string(&readme_path).map_err(|e| format!("{}: {e}", readme_path.display()))?;
    let want_hash =
        pinned(&readme, "tree-sha256").ok_or("third_party/README.md has no tree-sha256 line")?;
    let commit = pinned(&readme, "commit").ok_or("third_party/README.md has no commit line")?;
    let tag = pinned(&readme, "tag").ok_or("third_party/README.md has no tag line")?;
    let (hash, files) = tree_hash(&tree).map_err(|e| format!("{}: {e}", tree.display()))?;
    if hash != want_hash {
        return Err(format!(
            "the vendored whisper.cpp tree does not match its pin\n  pinned   {want_hash}\n  computed {hash} ({files} files)\n  tree     {}\nRestore the tree (git checkout third_party/whisper.cpp) or, for a deliberate update, run scripts/vendor-whisper-cpp.ps1 and update third_party/README.md.",
            tree.display()
        ));
    }
    println!("cargo:rustc-env=WB_WHISPER_TREE_SHA256={hash}");
    println!("cargo:rustc-env=WB_WHISPER_COMMIT={commit}");
    println!("cargo:rustc-env=WB_WHISPER_TAG={tag}");
    let variant = std::env::var("WB_ENGINE_VARIANT").unwrap_or_else(|_| "dev".into());
    println!("cargo:rustc-env=WB_ENGINE_VARIANT_NAME={variant}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            println!("cargo:warning=wb-engine build: {msg}");
            eprintln!("wb-engine build: {msg}");
            ExitCode::FAILURE
        }
    }
}
