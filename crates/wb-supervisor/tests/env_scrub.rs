//! A release-config supervisor (`scrub_env` on) starts an engine whose environment holds no `WB_*` variable,
//! although the supervisor's own environment has `WB_EMULATE` (and `WB_ENGINES_DIR`, `WB_MODELS_DIR`) and the
//! caller listed one in `SpawnSpec::env`. The control (`scrub_env` off, as debug and gate builds run) passes
//! them on. The child is `cmd /c set`, which prints its environment to stderr (the engine log file).
//!
//! The supervisor's own environment must hold the variables without `std::env::set_var` (unsafe, denied in
//! this workspace), so the test runs itself as a child with them set and does the real check there.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

use wb_supervisor::{EngineProcess, Priority, SpawnSpec};

const INNER: &str = "WB_SCRUB_TEST_INNER";

fn env_dump_of_child(scrub: bool, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("wb-env-scrub-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("engine.log");
    let cmd = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("System32")
        .join("cmd.exe");
    let spec = SpawnSpec {
        exe: cmd,
        args: vec!["/d".into(), "/c".into(), "set 1>&2".into()],
        priority: Priority::BelowNormal,
        stderr_log: log.clone(),
        env: vec![("WB_FROM_SPEC".into(), "1".into()), ("KEEP_ME".into(), "yes".into())],
        scrub_env: scrub,
    };
    let mut p = EngineProcess::spawn(&spec).expect("spawn cmd");
    let _ = p.wait();
    std::fs::read_to_string(&log).expect("child environment dump")
}

fn wb_lines(dump: &str) -> Vec<&str> {
    dump.lines()
        .filter(|l| l.to_ascii_uppercase().starts_with("WB_"))
        .collect()
}

#[cfg(windows)]
#[test]
fn a_release_config_supervisor_gives_its_child_no_wb_variables() {
    if std::env::var(INNER).is_ok() {
        // The supervisor process: its environment has WB_EMULATE, WB_ENGINES_DIR and WB_MODELS_DIR.
        assert!(std::env::var("WB_EMULATE").is_ok(), "the outer test sets WB_EMULATE");
        let scrubbed = env_dump_of_child(true, "on");
        assert!(wb_lines(&scrubbed).is_empty(), "WB_ variables reached the child: {:?}", wb_lines(&scrubbed));
        assert!(scrubbed.contains("KEEP_ME=yes"), "other variables still pass");
        // Control: the same supervisor with scrubbing off (debug and gate builds) passes them on.
        let kept = env_dump_of_child(false, "off");
        let names: Vec<String> = wb_lines(&kept)
            .iter()
            .map(|l| l.split('=').next().unwrap().to_ascii_uppercase())
            .collect();
        for want in ["WB_EMULATE", "WB_ENGINES_DIR", "WB_MODELS_DIR", "WB_FROM_SPEC"] {
            assert!(names.iter().any(|n| n == want), "control: {want} missing from {names:?}");
        }
        return;
    }
    let me = std::env::current_exe().unwrap();
    let out = Command::new(me)
        .args(["a_release_config_supervisor_gives_its_child_no_wb_variables", "--exact", "--nocapture"])
        .env(INNER, "1")
        .env("WB_EMULATE", "emu-low")
        .env("WB_ENGINES_DIR", "D:\\somewhere\\engines")
        .env("WB_MODELS_DIR", "D:\\somewhere\\models")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "inner test failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_default_policy_scrubs_in_release_builds_and_not_in_debug_builds() {
    assert_eq!(wb_supervisor::Policy::default().scrub_env, !cfg!(debug_assertions));
}

#[test]
fn the_wb_prefix_test_never_panics_on_non_ascii_names() {
    use wb_supervisor::process::is_wb_variable;
    for name in ["WB_EMULATE", "wb_emulate", "Wb_X", "WB_"] {
        assert!(is_wb_variable(name), "{name}");
    }
    // A three-byte character straddling byte 3 used to panic in the string slice.
    for name in ["ab\u{20ac}", "a\u{20ac}", "\u{20ac}", "W", "WB", "", "WA_X", "\u{0448}\u{0431}_X"] {
        assert!(!is_wb_variable(name), "{name:?}");
    }
}
