//! The engine builds and the model files this computer has, for the System screen and (later) the
//! scheduler.
//!
//! * Engines: each `wb-engine-<variant>` next to the app (or in `WB_ENGINES_DIR`, or the repository's
//!   `src-tauri/binaries` in development) is started once with `info` in its own process, with a
//!   timeout. A build that crashes, lacks a CPU feature (exit code 3) or finds no device is marked and
//!   the app carries on: engines are separate processes (PLAN section 7).
//! * Models: `models/manifest.json` lists the files; each is hashed on a low-priority thread and
//!   marked missing, truncated or altered when it does not match. Nothing is ever downloaded.
//!
//! Nothing here is shared with the recorder thread, and a failure here only produces a Toast.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Manager;
use wb_models::{Cache, Manifest, Status};
use wb_protocol::{BackendDevice, CpuReport, Event as EngineEvent, WhisperCppInfo};
use wb_recorder::events::{Notice, NoticeLevel};
use wb_supervisor::{EngineProcess, Line, Priority, SpawnSpec};

use crate::boundary;
use crate::state::{AppState, lock};

/// Engine builds the app knows, in the order the system check prefers to try them.
pub const VARIANTS: &[&str] = &["avx2", "avx512", "vulkan", "cuda", "openvino"];
const INFO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Checking,
    /// `info` succeeded.
    Ok,
    /// The CPU lacks an instruction set this build was compiled for (exit code 3).
    CpuMismatch,
    /// It ran but failed, crashed or printed nothing useful.
    Failed,
    /// It did not answer within the timeout.
    Timeout,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineRow {
    pub variant: String,
    pub file: String,
    pub bytes: u64,
    pub state: EngineState,
    pub message: Option<String>,
    pub devices: Vec<BackendDevice>,
    pub whisper_cpp: Option<WhisperCppInfo>,
    pub cpu: Option<CpuReport>,
    pub backends: Vec<String>,
    pub info_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelState {
    Checking,
    Ok,
    Missing,
    SizeMismatch { expected: u64, actual: u64 },
    HashMismatch,
    NoChecksum,
}

impl From<&Status> for ModelState {
    fn from(s: &Status) -> Self {
        match s {
            Status::Ok => Self::Ok,
            Status::Missing => Self::Missing,
            Status::SizeMismatch { expected, actual } => Self::SizeMismatch {
                expected: *expected,
                actual: *actual,
            },
            Status::HashMismatch { .. } => Self::HashMismatch,
            Status::NoChecksum => Self::NoChecksum,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelRow {
    pub id: String,
    pub file: String,
    pub bytes: u64,
    pub base: String,
    pub quant: String,
    pub languages: Vec<String>,
    pub candidate_roles: Vec<String>,
    pub validated_roles: Vec<String>,
    pub licence: String,
    #[serde(flatten)]
    pub state: ModelState,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RegistrySnapshot {
    pub engines_dir: Option<String>,
    pub engines: Vec<EngineRow>,
    pub engines_done: bool,
    pub models_dir: Option<String>,
    pub models: Vec<ModelRow>,
    pub models_done: bool,
    /// Why the manifest could not be read, when it could not.
    pub manifest_error: Option<String>,
}

#[derive(Default)]
pub struct Registry {
    inner: Mutex<RegistrySnapshot>,
}

impl Registry {
    pub fn snapshot(&self) -> RegistrySnapshot {
        lock(&self.inner).clone()
    }
}

// ---------------------------------------------------------------------------------- locating

/// Folders to look in, nearest first: the app's own folder, then (development) the repository.
fn exe_ancestors() -> Vec<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|p| {
            p.ancestors()
                .skip(1)
                .take(6)
                .map(Path::to_path_buf)
                .collect()
        })
        .unwrap_or_default()
}

fn has_engines(dir: &Path) -> bool {
    !discover(dir).is_empty()
}

pub fn locate_engines_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("WB_ENGINES_DIR") {
        return Some(PathBuf::from(d));
    }
    for dir in exe_ancestors() {
        if has_engines(&dir) {
            return Some(dir);
        }
        let dev = dir.join("src-tauri").join("binaries");
        if has_engines(&dev) {
            return Some(dev);
        }
    }
    None
}

pub fn locate_models_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("WB_MODELS_DIR") {
        return Some(PathBuf::from(d));
    }
    exe_ancestors()
        .into_iter()
        .map(|d| d.join("models"))
        .find(|m| m.join("manifest.json").is_file())
}

/// `(variant, file)` for every engine executable in `dir`, in preference order.
pub fn discover(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for v in VARIANTS {
        for name in [
            format!("wb-engine-{v}.exe"),
            format!("wb-engine-{v}-x86_64-pc-windows-msvc.exe"),
        ] {
            let p = dir.join(&name);
            if p.is_file() {
                out.push(((*v).to_string(), p));
                break;
            }
        }
    }
    out
}

// -------------------------------------------------------------------------------------- probing

/// Starts `exe info` and reads what it says, with a timeout.
pub fn probe_engine(variant: &str, exe: &Path, log_dir: &Path) -> EngineRow {
    let bytes = std::fs::metadata(exe).map(|m| m.len()).unwrap_or(0);
    let mut row = EngineRow {
        variant: variant.to_string(),
        file: exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        bytes,
        state: EngineState::Failed,
        message: None,
        devices: Vec::new(),
        whisper_cpp: None,
        cpu: None,
        backends: Vec::new(),
        info_ms: 0,
    };
    let spec = SpawnSpec {
        exe: exe.to_path_buf(),
        args: vec!["info".into()],
        priority: Priority::Normal,
        stderr_log: log_dir.join(format!("info-{variant}.log")),
        env: Vec::new(),
    };
    let started = Instant::now();
    let mut proc = match EngineProcess::spawn(&spec) {
        Ok(p) => p,
        Err(e) => {
            row.message = Some(format!("could not start: {e}"));
            return row;
        }
    };
    let mut error: Option<(String, String)> = None;
    loop {
        let left = INFO_TIMEOUT.saturating_sub(started.elapsed());
        if left.is_zero() {
            proc.kill();
            row.state = EngineState::Timeout;
            row.message = Some(format!("no answer within {} s", INFO_TIMEOUT.as_secs()));
            row.info_ms = started.elapsed().as_millis() as u64;
            return row;
        }
        match proc
            .events
            .recv_timeout(left.min(Duration::from_millis(500)))
        {
            Ok(Line::Event(e)) => match *e {
                EngineEvent::Hello(h) => row.whisper_cpp = h.whisper_cpp,
                EngineEvent::Info(report) => {
                    row.whisper_cpp = Some(report.whisper_cpp.clone());
                    row.devices = report.devices.clone();
                    row.backends = report.backends.clone();
                    row.cpu = Some(report.cpu.clone());
                    row.message = report.notes.first().cloned();
                    row.state = EngineState::Ok;
                }
                EngineEvent::Error { code, message, .. } => error = Some((code, message)),
                _ => {}
            },
            Ok(Line::Garbled(_)) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let exit = proc.wait();
    row.info_ms = started.elapsed().as_millis() as u64;
    if let Some((code, message)) = error {
        row.state = if code == "cpu_features" {
            EngineState::CpuMismatch
        } else {
            EngineState::Failed
        };
        row.message = Some(message);
    } else if !matches!(row.state, EngineState::Ok) {
        row.message = Some(format!(
            "exited with code {} without a report",
            exit.map_or("?".into(), |c| c.to_string())
        ));
    }
    row
}

// ----------------------------------------------------------------------------------- refresh

fn notice(state: &AppState, level: NoticeLevel, message: String) {
    let _ = state.events.send(Notice::event(level, "engines", message));
}

/// Probes every engine build found, one after the other, updating the registry as it goes.
pub fn refresh_engines(registry: &Registry, state: &AppState) {
    let dir = locate_engines_dir();
    let found = dir.as_deref().map(discover).unwrap_or_default();
    {
        let mut r = lock(&registry.inner);
        r.engines_dir = dir.as_ref().map(|d| d.to_string_lossy().into_owned());
        r.engines_done = false;
        r.engines = found
            .iter()
            .map(|(v, p)| EngineRow {
                variant: v.clone(),
                file: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                state: EngineState::Checking,
                message: None,
                devices: Vec::new(),
                whisper_cpp: None,
                cpu: None,
                backends: Vec::new(),
                info_ms: 0,
            })
            .collect();
    }
    if found.is_empty() {
        lock(&registry.inner).engines_done = true;
        notice(
            state,
            NoticeLevel::Warning,
            match &dir {
                Some(d) => format!("No speech engine found in {}. Recording is not affected; sessions are recorded and transcribed later, on a computer that has the engines.", d.display()),
                None => "No speech engine found next to the app. Recording is not affected; sessions are recorded and transcribed later, on a computer that has the engines.".into(),
            },
        );
        return;
    }
    let log_dir = state.data_dir.join("engine-logs");
    let mut usable = 0;
    for (i, (variant, exe)) in found.iter().enumerate() {
        let row = probe_engine(variant, exe, &log_dir);
        if matches!(row.state, EngineState::Ok) {
            usable += 1;
        } else {
            notice(
                state,
                NoticeLevel::Warning,
                format!(
                    "The {variant} speech engine cannot be used on this computer: {}",
                    row.message.clone().unwrap_or_else(|| "it failed".into())
                ),
            );
        }
        if let Some(slot) = lock(&registry.inner).engines.get_mut(i) {
            *slot = row;
        }
    }
    lock(&registry.inner).engines_done = true;
    if usable == 0 {
        notice(state, NoticeLevel::Error, "None of the speech engines can run on this computer. Recording is not affected; sessions are transcribed later on another computer.".into());
    }
}

fn model_row(e: &wb_models::Entry, state: ModelState, message: Option<String>) -> ModelRow {
    ModelRow {
        id: e.id.clone(),
        file: e.file.clone(),
        bytes: e.bytes,
        base: e.base.clone(),
        quant: e.quant.clone(),
        languages: e.languages.clone(),
        candidate_roles: e.candidate_roles.clone(),
        validated_roles: e.validated_roles.clone(),
        licence: e.licence.clone(),
        state,
        message,
    }
}

/// Hashes every model in the manifest, updating the registry after each. Slow (gigabytes): run it
/// on a thread of its own at low priority.
pub fn refresh_models(registry: &Registry, state: &AppState) {
    let dir = locate_models_dir();
    let Some(dir) = dir else {
        let mut r = lock(&registry.inner);
        r.models_done = true;
        r.manifest_error =
            Some("No models folder with a manifest.json was found next to the app.".into());
        return;
    };
    let manifest = match Manifest::load(&dir.join("manifest.json")) {
        Ok(m) => m,
        Err(e) => {
            let mut r = lock(&registry.inner);
            r.models_dir = Some(dir.to_string_lossy().into_owned());
            r.models_done = true;
            r.manifest_error = Some(e.to_string());
            notice(
                state,
                NoticeLevel::Warning,
                format!("The model list could not be read: {e}"),
            );
            return;
        }
    };
    {
        let mut r = lock(&registry.inner);
        r.models_dir = Some(dir.to_string_lossy().into_owned());
        r.manifest_error = None;
        r.models_done = false;
        r.models = manifest
            .models
            .iter()
            .map(|e| model_row(e, ModelState::Checking, None))
            .collect();
    }
    let cache_path = state.data_dir.join("verified-models.json");
    let mut cache = Cache::load(&cache_path);
    for (i, entry) in manifest.models.iter().enumerate() {
        let mut hashed = false;
        let status = wb_models::verify(&dir, entry, &mut cache, &mut hashed);
        let message = (!status.is_ok()).then(|| status.describe());
        if !status.is_ok() && !matches!(status, Status::NoChecksum | Status::Missing) {
            notice(
                state,
                NoticeLevel::Error,
                format!("Model {} is disabled: {}", entry.id, status.describe()),
            );
        }
        if let Some(slot) = lock(&registry.inner).models.get_mut(i) {
            *slot = model_row(entry, ModelState::from(&status), message);
        }
        if hashed {
            let _ = cache.save(&cache_path);
        }
    }
    lock(&registry.inner).models_done = true;
}

/// Starts the engine probe and the model check on threads of their own. They never block start-up
/// or the recorder, and a panic in either ends only that thread.
pub fn start(app: &tauri::AppHandle) {
    for (name, models) in [("wb-engine-probe", false), ("wb-model-check", true)] {
        let handle = app.clone();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                wb_recorder::sys::lower_current_thread();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let state = handle.state::<AppState>();
                    let registry = handle.state::<Registry>();
                    if models {
                        refresh_models(&registry, &state);
                    } else {
                        refresh_engines(&registry, &state);
                    }
                }));
                if let Err(p) = result {
                    tracing::error!(
                        thread = name,
                        "registry thread panicked: {}",
                        boundary::panic_message(p.as_ref())
                    );
                }
            });
        if let Err(e) = spawned {
            tracing::error!("could not start {name}: {e}");
        }
    }
}

/// What the System screen shows.
#[tauri::command]
pub fn list_engines(registry: tauri::State<'_, Registry>) -> RegistrySnapshot {
    registry.snapshot()
}

/// Looks again (after a model pack was copied in, or an engine file was added).
#[tauri::command]
pub fn recheck_registry(app: tauri::AppHandle) {
    start(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-engines-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        d
    }

    #[test]
    fn engines_are_found_by_either_file_name_in_preference_order() {
        let d = temp("discover");
        std::fs::write(d.join("wb-engine-vulkan-x86_64-pc-windows-msvc.exe"), b"x").expect("write");
        std::fs::write(d.join("wb-engine-avx2.exe"), b"x").expect("write");
        std::fs::write(d.join("wb-engine-unknown.exe"), b"x").expect("write");
        std::fs::write(d.join("notes.txt"), b"x").expect("write");
        let found = discover(&d);
        assert_eq!(
            found.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(),
            vec!["avx2", "vulkan"]
        );
        assert!(discover(&d.join("nowhere")).is_empty());
    }

    #[test]
    fn an_engine_that_cannot_be_started_is_a_failed_row_not_an_error() {
        let d = temp("probe");
        let missing = d.join("wb-engine-avx2.exe");
        std::fs::write(&missing, b"this is not an executable").expect("write");
        let row = probe_engine("avx2", &missing, &d.join("logs"));
        assert!(matches!(row.state, EngineState::Failed), "{row:?}");
        assert!(row.message.is_some());
    }

    #[test]
    fn model_states_follow_the_checksum_result() {
        assert!(matches!(ModelState::from(&Status::Ok), ModelState::Ok));
        assert!(matches!(
            ModelState::from(&Status::Missing),
            ModelState::Missing
        ));
        assert!(matches!(
            ModelState::from(&Status::HashMismatch {
                expected: "a".into(),
                actual: "b".into()
            }),
            ModelState::HashMismatch
        ));
        assert!(matches!(
            ModelState::from(&Status::SizeMismatch {
                expected: 2,
                actual: 1
            }),
            ModelState::SizeMismatch {
                expected: 2,
                actual: 1
            }
        ));
    }
}
