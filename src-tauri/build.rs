/// The app's own commands. Capabilities must grant each one explicitly.
const COMMANDS: &[&str] = &[
    "has_capability_profile",
    "list_inputs",
    "get_settings",
    "save_settings",
    "start_session",
    "pause",
    "resume",
    "stop_session",
    "subscribe_events",
    "recorder_snapshot",
    "recovery_list",
    "recover_session",
    "list_engines",
    "recheck_registry",
    "get_capability",
    "run_system_check",
    "cancel_system_check",
    "set_override",
    "session_plan",
];

fn main() {
    // The gate-hooks feature (environment-variable test hooks) must never reach an installer.
    // Allowlist: it builds only with `--profile gate` (OUT_DIR is <target>\gate\build\...), and
    // never under the Tauri CLI, which is what makes installers (release or --debug).
    if std::env::var_os("CARGO_FEATURE_GATE_HOOKS").is_some() {
        let out = std::env::var("OUT_DIR").unwrap_or_default();
        let profile_dir = std::path::Path::new(&out)
            .ancestors()
            .find(|p| p.file_name() == Some("build".as_ref()))
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        let under_tauri_cli = [
            "TAURI_ENV_PLATFORM",
            "TAURI_ENV_DEBUG",
            "TAURI_ENV_TARGET_TRIPLE",
        ]
        .iter()
        .any(|k| std::env::var_os(k).is_some());
        if profile_dir.as_deref() != Some("gate") || under_tauri_cli {
            eprintln!(
                "error: the gate-hooks feature builds only with plain cargo and --profile gate (scripts/gates/build-gate.ps1), never in an installer build"
            );
            std::process::exit(1);
        }
    }
    for var in [
        "TAURI_ENV_PLATFORM",
        "TAURI_ENV_DEBUG",
        "TAURI_ENV_TARGET_TRIPLE",
        "CARGO_FEATURE_GATE_HOOKS",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(err) = tauri_build::try_build(attributes) {
        eprintln!("tauri-build failed: {err:#}");
        std::process::exit(1);
    }
}
