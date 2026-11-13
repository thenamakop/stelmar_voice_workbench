// Typed wrappers around the app's Tauri commands (src-tauri/src). Every call can fail; callers
// surface the error with a Toast. Field names follow the Rust types (snake_case).
import { Channel, invoke } from "@tauri-apps/api/core";

/** True when the first-launch system check has saved capability.json on this machine. */
export function hasCapabilityProfile(): Promise<boolean> {
  return invoke<boolean>("has_capability_profile");
}

export interface InputDevice {
  id: string;
  name: string;
  is_default: boolean;
  sample_rate: number;
  channels: number;
  sample_format: string;
}

export interface InputInfo {
  device_name: string;
  device_id: string;
  sample_rate: number;
  channels: number;
  sample_format: string;
}

export interface Settings {
  recordings_root: string;
  backup_root: string | null;
  last_device_id: string | null;
}

export type RecState = "recording" | "paused" | "input_lost" | "stopped";

export interface RecorderStatus {
  state: RecState;
  session_dir: string;
  audio_ms: number;
  part: number | null;
  part_audio_ms: number;
  part_seconds: number;
  overrun_samples: number;
  xruns: number;
  dropped_samples: number;
  recorder_restarts: number;
  position_breaks: number;
  timestamp_outliers: number;
  position_pair_flags: number;
  repeated_blocks: number;
  zero_gaps: number;
  ring_seconds: number;
  ring_max_fill_pct: number;
  priority: "mmcss" | "highest" | "normal";
  priority_note: string | null;
  capture_priority: string | null;
  awake: boolean;
  disk_free_bytes: number | null;
  disk_warning: boolean;
  input: InputInfo | null;
  input_lost_since: string | null;
  thread_cpu_ms: number | null;
  last_error: string | null;
}

export type MirrorState = "not_set" | "in_sync" | "behind" | "missing" | "error";

export interface MirrorStatus {
  state: MirrorState;
  backup_dir: string | null;
  lag_ms: number;
  warn: boolean;
  last_sync_at: string | null;
  message: string | null;
}

export interface PartRecord {
  part: number;
  file: string;
  start_ms: number;
  duration_ms: number;
  frames: number;
  sample_rate: number;
  channels: number;
  sha256: string;
  /** Set when the header did not match the file length at close; recovery rewrites it. */
  needs_repair?: boolean;
}

export interface SessionInfo {
  dir: string;
  title: string;
  hall: string | null;
  scheduled: string | null;
  expected_languages: string[];
  mode: string;
  opened_at: string;
  input: InputInfo;
  backup_root: string | null;
  resumed: boolean;
}

export interface MirrorVerify {
  backup_dir: string | null;
  files: { file: string; sha256: string; mirror_sha256: string | null; matches: boolean }[];
  all_match: boolean;
  message: string | null;
}

export interface SessionStopped {
  recorder: {
    session_dir: string;
    parts: PartRecord[];
    audio_ms: number;
    overrun_samples: number;
    xruns: number;
    recorder_cpu_ms: number | null;
    manifest_files: number;
  };
  mirror: MirrorVerify;
}

export interface Snapshot {
  session: SessionInfo | null;
  recorder: RecorderStatus | null;
  mirror: MirrorStatus | null;
  parts: PartRecord[];
  last_stop: SessionStopped | null;
  display_restarts: number;
  display_warning: string | null;
}

export interface LevelReading {
  rms_dbfs: number;
  peak_dbfs: number;
  clipped: boolean;
}

export type NoticeLevel = "info" | "warning" | "error";

/** Events on the channel from `subscribe_events`. */
export type AppEvent =
  | ({ kind: "level" } & LevelReading)
  | ({ kind: "recorder_status" } & RecorderStatus)
  | ({ kind: "mirror_status" } & MirrorStatus)
  | ({ kind: "part_closed" } & PartRecord)
  | { kind: "notice"; level: NoticeLevel; source: string; message: string }
  | { kind: "display_recovered"; restarts: number; failure: string }
  | { kind: "close_blocked" }
  | { kind: "check_progress"; progress: CheckProgress }
  | { kind: "check_done" }
  | { kind: "check_failed"; message: string };

export interface StartRequest {
  title: string;
  hall?: string | null;
  programme_id?: string | null;
  scheduled?: string | null;
  expected_languages?: string[];
  device_id?: string | null;
  /** `record_post` (default) or `live_post`; live is accepted only where the saved profile clears it. */
  mode?: string;
  /** How Hindi and Urdu are written: the organiser's choice, kept beside the session. */
  hindi_urdu_script?: HindiUrduScript;
}

export interface RecoveredSession {
  dir: string;
  title: string;
  hall: string | null;
  opened_at: string;
  audio_ms: number;
  parts: number;
  repairs: { file: string; already_valid: boolean; truncated_bytes: number; frames: number }[];
  removed_empty: string[];
  quarantined: string[];
}

export interface RecoveryList {
  done: boolean;
  sessions: RecoveredSession[];
  /** Sessions a crash left open that could not be repaired (folder and reason). */
  failed: string[];
}

export const listInputs = () => invoke<InputDevice[]>("list_inputs");
export const getSettings = () => invoke<Settings>("get_settings");
export const saveSettings = (settings: Settings) => invoke<Settings>("save_settings", { settings });
export const startSession = (request: StartRequest) => invoke<SessionInfo>("start_recording", { request });
export const pauseRecording = () => invoke<void>("pause");
export const resumeRecording = () => invoke<void>("resume");
export const stopSession = () => invoke<SessionStopped>("stop_session");
export const recorderSnapshot = () => invoke<Snapshot>("recorder_snapshot");
export const recoveryList = () => invoke<RecoveryList>("recovery_list");
export const recoverSession = (dir: string, action: "resume" | "close") =>
  invoke<SessionInfo | null>("recover_session", { dir, action });

/** Subscribes to recorder, mirror and display events for the life of the page. */
export function subscribeEvents(onEvent: (event: AppEvent) => void): Promise<void> {
  const channel = new Channel<AppEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("subscribe_events", { onEvent: channel });
}

// ---- Engines and models (System screen) ---------------------------------------------------------

export type EngineStateName = "checking" | "ok" | "cpu_mismatch" | "failed" | "timeout";

export interface EngineDevice {
  index: number;
  backend: string;
  name: string;
  description: string;
  kind: "cpu" | "gpu" | "igpu" | "accel" | "other";
  total_memory_bytes: number;
  free_memory_bytes: number;
  driver_version: string | null;
  duplicates_dropped: number;
  shared_memory_bytes: number | null;
  adapter_name: string | null;
}

export interface EngineRow {
  variant: string;
  file: string;
  bytes: number;
  state: EngineStateName;
  message: string | null;
  devices: EngineDevice[];
  whisper_cpp: { version: string; commit: string; tree_sha256: string } | null;
  backends: string[];
  info_ms: number;
}

export type ModelStateName =
  "checking" | "ok" | "missing" | "size_mismatch" | "hash_mismatch" | "no_checksum";

export interface ModelRow {
  id: string;
  file: string;
  bytes: number;
  base: string;
  quant: string;
  languages: string[];
  candidate_roles: string[];
  validated_roles: string[];
  licence: string;
  state: ModelStateName;
  message: string | null;
}

export interface RegistrySnapshot {
  engines_dir: string | null;
  engines: EngineRow[];
  engines_done: boolean;
  /** Whether a speech engine can be used here; the app state the Record screen reads on every load. */
  speech_engine: "checking" | "available" | "none_found" | "none_usable";
  models_dir: string | null;
  models: ModelRow[];
  models_done: boolean;
  manifest_error: string | null;
}

export const listEngines = () => invoke<RegistrySnapshot>("list_engines");
export const recheckRegistry = () => invoke<void>("recheck_registry");

// ---- System check and capability.json (Phase 3) ---------------------------------------------------
// Shapes follow crates/wb-capability/src/types.rs. Sizes are bytes, speeds are real-time factors
// (processing time / audio time; lower is faster).

export type CapLang = "en" | "hi" | "ur";
export type Tier = "G" | "C+" | "C" | "R";
export type DeviceConfig = { kind: "cpu" } | { kind: "gpu"; index: number } | { kind: "hybrid"; gpu: number };
export type OverrideKind = "process_while_recording" | "pause_on_battery" | "allow_live_anyway";
export type HindiUrduScript = "devanagari" | "nastaliq";

export interface CapGpu {
  index: number;
  name: string;
  vendor: string;
  kind: "integrated" | "discrete";
  dedicated_bytes: number;
  local_bytes: number;
  shared_bytes: number;
  shared_assumed: boolean;
  driver: string | null;
  vulkan_usable: boolean;
}

export interface CapHardware {
  os: string;
  cpu_brand: string;
  logical_processors: number;
  physical_cores: number;
  cpu_features: string[];
  avx2: boolean;
  avx512: boolean;
  total_ram_bytes: number;
  available_ram_bytes: number;
  gpus: CapGpu[];
  disks: { role: string; path: string; free_bytes: number }[];
  on_mains_power: boolean | null;
  audio_inputs: number;
}

export interface CapMeasurement {
  model: string;
  lang: CapLang;
  mode: "rolling" | "near_live";
  config: DeviceConfig;
  engine_build: string;
  device_name: string;
  threads: number;
  method: string;
  clip_seconds: number;
  rtf: number;
  warmup_rtf: number | null;
  load_s: number;
  peak_working_set_bytes: number;
  peak_commit_bytes: number;
  gpu_local_bytes: number | null;
  truncated_chunks: number;
  live_standin?: boolean;
}

export interface RollingPick {
  model: string;
  rtf: number;
  meets_bar: boolean;
  quality_unverified: boolean;
  k_max: number;
  ready_after_stop_min: number;
  lags: boolean;
}

export interface NightPick {
  model: string;
  rtf: number;
  hours_for_a_day: number;
  quality_unverified: boolean;
}

export interface LiveDecision {
  offered: boolean;
  kind: string | null;
  model: string | null;
  rtf: number | null;
  soft_reasons: string[];
  hard_reasons: string[];
}

export interface LangDecision {
  rolling: RollingPick | null;
  night: NightPick | null;
  night_note: string | null;
  live: LiveDecision;
}

export interface CapDecision {
  tier: Tier;
  tier_reasons: string[];
  record_only_reason: string | null;
  bars: { rolling_rtf: number; near_live_rtf: number; live_standin_rtf: number; headroom_factor: number };
  config: {
    config: DeviceConfig;
    label: string;
    why: string;
    cpu_threads: number;
    recorder_safety_passed: boolean | null;
  } | null;
  languages: Partial<Record<CapLang, LangDecision>>;
  hinglish: { ok: boolean; rtf: number | null; source: string };
  dual_decode_en_hi: boolean;
  provisional: boolean;
  partial: boolean;
  verdict: string[];
  process_while_recording: boolean;
  pause_on_battery: boolean;
}

export interface Capability {
  schema_version: number;
  app_version: string;
  created_at: string;
  emulated: string | null;
  fingerprint: { os: string; cpu_brand: string };
  check: {
    kind: "quick" | "full";
    partial: boolean;
    started_at: string;
    duration_s: number;
    bar_s: number;
    excerpt_seconds: number;
  };
  hardware: CapHardware;
  engines: { variant: string; usable: boolean; reason: string | null; info_ms: number | null; devices: string[] }[];
  measurements: CapMeasurement[];
  hybrid: {
    model: string;
    lang: CapLang;
    gpu: number;
    cpu_threads: number;
    gpu_rtf_concurrent: number;
    cpu_rtf_concurrent: number;
    method: string;
  } | null;
  recorder_safety: {
    config: DeviceConfig;
    duration_s: number;
    overruns: number;
    driver_xruns: number;
    passed: boolean;
  }[];
  injectors: string[];
  skipped?: { step: string; reason: string }[];
  decision: CapDecision;
  overrides: { kind: OverrideKind; enabled: boolean; at: string; confirmed: boolean }[];
}

/** One saved check and the file it lives in (a partial or cancelled check never replaces a complete profile). */
export interface SavedCheck {
  file: "main" | "partial";
  started_at: string;
  kind: "quick" | "full";
  tier: Tier;
  partial: boolean;
  duration_s: number;
  fingerprint_id: string;
  /** The profile the rest of the app reads. */
  in_use: boolean;
}

export interface CapabilityView {
  /** The profile in use; after a fingerprint change it carries safe defaults (record + post-process only). */
  capability: Capability | null;
  saved_checks: SavedCheck[];
  error: string | null;
  running: boolean;
  fingerprint_changes: string[];
  fingerprint_id: string | null;
  development: boolean;
}

export interface SessionPlan {
  default_mode: string;
  record_only: boolean;
  record_only_reason: string | null;
  live: { selectable: boolean; reason: string | null; via_override: boolean };
  devices: string;
  models: { lang: CapLang; model: string; quality_unverified: boolean }[];
  ready_after_stop_min: number | null;
  lags: boolean;
  provisional: boolean;
  partial: boolean;
}

export interface CheckProgress {
  phase: string;
  detail: string;
  elapsed_s: number;
  cells_done: number;
}

export const getCapability = () => invoke<CapabilityView>("get_capability");
export const runSystemCheck = (full: boolean) => invoke<void>("run_system_check", { full });
export const cancelSystemCheck = () => invoke<void>("cancel_system_check");
export const setOverride = (kind: OverrideKind, enabled: boolean, confirmed: boolean) =>
  invoke<CapabilityView>("set_override", { kind, enabled, confirmed });
export const sessionPlan = (english: boolean, hindiUrdu: boolean, script: HindiUrduScript) =>
  invoke<SessionPlan | null>("session_plan", { english, hindiUrdu, script });
