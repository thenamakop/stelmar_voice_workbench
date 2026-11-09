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
  | { kind: "close_blocked" };

export interface StartRequest {
  title: string;
  hall?: string | null;
  programme_id?: string | null;
  scheduled?: string | null;
  expected_languages?: string[];
  device_id?: string | null;
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
export const startSession = (request: StartRequest) => invoke<SessionInfo>("start_session", { request });
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
