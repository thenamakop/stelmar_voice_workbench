// A mocked Tauri bridge for Vitest and the Playwright build (src/test/e2e-entry.ts): answers every
// app command with fixture data, so screens render as they would on a machine. Fixture numbers
// are placeholders, like the mockups'.
import type {
  Capability,
  HindiUrduScript,
  InputDevice,
  OverrideKind,
  MirrorStatus,
  PartRecord,
  RecorderStatus,
  RecoveredSession,
  SavedCheck,
  SessionInfo,
  Settings,
  Snapshot,
} from "../ipc";
import { planFor, profile, safeDefaults } from "./profiles";

export type MockState = "idle" | "recording" | "input_lost" | "recovery";

export interface MockOptions {
  /** capability.json exists (or the lookup fails with this error). */
  profile?: boolean | Error;
  state?: MockState;
  /** "none": every speech engine renamed away (list_engines finds no engine). Default: engines present. */
  engines?: "present" | "none";
  /** A saved capability profile from crates/wb-capability/fixtures (src/test/profiles.ts); none by default. */
  capability?: string;
  /** The saved profile no longer matches this machine (fingerprint-change banner). */
  fingerprintChange?: string[];
  /** The system check is running right now. */
  checkRunning?: boolean;
  /** A partial check kept apart from the complete profile (a fixture name from src/test/profiles.ts). */
  partialCapability?: string;
  /** Commands seen, in order (for assertions). */
  calls?: string[];
}

export const INPUTS: InputDevice[] = [
  {
    id: "wasapi:{line-1}",
    name: "USB interface · Line 1",
    is_default: true,
    sample_rate: 48000,
    channels: 2,
    sample_format: "f32",
  },
  {
    id: "wasapi:{mic}",
    name: "Microphone Array",
    is_default: false,
    sample_rate: 48000,
    channels: 2,
    sample_format: "f32",
  },
];

export const SETTINGS: Settings = {
  recordings_root: "D:\\STELMAR-Transcripts",
  backup_root: "E:\\STELMAR-Backup",
  last_device_id: "wasapi:{line-1}",
};

const DIR = "D:\\STELMAR-Transcripts\\2027-02-08\\seminar-hall-b__parallel-track-sessions-i";

export const SESSION: SessionInfo = {
  dir: DIR,
  title: "Parallel Track Sessions I",
  hall: "Seminar Hall B",
  scheduled: "08 FEB 2027 · 12:00–13:30 IST",
  expected_languages: ["en", "hi"],
  mode: "record_post",
  opened_at: "2027-02-08T11:58:02+05:30",
  input: {
    device_name: "USB interface · Line 1",
    device_id: "wasapi:{line-1}",
    sample_rate: 48000,
    channels: 2,
    sample_format: "f32",
  },
  backup_root: "E:\\STELMAR-Backup",
  resumed: false,
};

const PART_MS = 600_000;
export const PARTS: PartRecord[] = [1, 2, 3, 4].map((n) => ({
  part: n,
  file: `audio/part-000${n}.wav`,
  start_ms: (n - 1) * PART_MS,
  duration_ms: PART_MS,
  frames: 28_800_000,
  sample_rate: 48000,
  channels: 2,
  sha256: "0".repeat(64),
}));

export function recorderStatus(state: RecorderStatus["state"] = "recording"): RecorderStatus {
  return {
    state,
    session_dir: DIR,
    audio_ms: 2_715_000,
    part: state === "recording" ? 5 : null,
    part_audio_ms: state === "recording" ? 315_000 : 0,
    part_seconds: 600,
    overrun_samples: 0,
    xruns: 0,
    dropped_samples: 0,
    recorder_restarts: 0,
    position_breaks: 0,
    timestamp_outliers: 0,
    position_pair_flags: 0,
    repeated_blocks: 0,
    zero_gaps: 0,
    ring_seconds: 20,
    ring_max_fill_pct: 0.4,
    priority: "mmcss",
    priority_note: null,
    capture_priority: "mmcss",
    awake: true,
    disk_free_bytes: 388e9,
    disk_warning: false,
    input: SESSION.input,
    input_lost_since: state === "input_lost" ? "2027-02-08T12:43:10+05:30" : null,
    thread_cpu_ms: 2_100,
    last_error: null,
  };
}

export const MIRROR: MirrorStatus = {
  state: "in_sync",
  backup_dir: "E:\\STELMAR-Backup\\2027-02-08\\seminar-hall-b__parallel-track-sessions-i",
  lag_ms: 0,
  warn: false,
  last_sync_at: "2027-02-08T12:43:17+05:30",
  message: null,
};

export const RECOVERED: RecoveredSession = {
  dir: DIR,
  title: "Parallel Track Sessions I",
  hall: "Seminar Hall B",
  opened_at: "2027-02-08T11:58:02+05:30",
  audio_ms: 2_473_000,
  parts: 5,
  repairs: [{ file: "part-0005.wav", already_valid: false, truncated_bytes: 2, frames: 3_504_000 }],
  removed_empty: [],
  quarantined: [],
};

function snapshot(state: MockState): Snapshot {
  const base: Snapshot = {
    session: null,
    recorder: null,
    mirror: null,
    parts: [],
    last_stop: null,
    display_restarts: 0,
    display_warning: null,
  };
  if (state === "recording" || state === "input_lost") {
    return {
      ...base,
      session: SESSION,
      recorder: recorderStatus(state === "input_lost" ? "input_lost" : "recording"),
      mirror: MIRROR,
      parts: state === "input_lost" ? PARTS.slice(0, 4) : PARTS,
    };
  }
  return base;
}

type ChannelLike = { id: number };

function emit(channel: ChannelLike, messages: unknown[], first = 0) {
  const internals = (
    window as unknown as { __TAURI_INTERNALS__?: { runCallback?: (id: number, data: unknown) => void } }
  ).__TAURI_INTERNALS__;
  messages.forEach((message, i) => internals?.runCallback?.(channel.id, { index: first + i, message }));
}

/** The mockIPC handler. */
export function bridge(opts: MockOptions = {}) {
  const state = opts.state ?? "idle";
  let recording = state === "recording" || state === "input_lost";
  // A bare `profile: true` stands for "a profile exists": the low-baseline one.
  const cap: Capability | null = opts.capability
    ? profile(opts.capability)
    : opts.profile === true
      ? profile("tier-c-low")
      : null;
  const partialBase = opts.partialCapability ? profile(opts.partialCapability) : null;
  const partialCap: Capability | null = partialBase
    ? { ...partialBase, check: { ...partialBase.check, partial: true } }
    : null;
  const changed = (opts.fingerprintChange ?? []).length > 0;
  // After a fingerprint change the app reads the profile with safe defaults (wb_capability::safe_defaults).
  const inUse: Capability | null =
    cap && changed ? safeDefaults(cap, `This computer has changed since the check (${opts.fingerprintChange?.[0]}). Safe defaults apply: recording and post-processing only, until a new check has run.`) : cap;
  const summary = (c: Capability, file: "main" | "partial", used: boolean): SavedCheck => ({
    file,
    started_at: c.check.started_at,
    kind: c.check.kind,
    tier: c.decision.tier,
    partial: c.check.partial,
    duration_s: c.check.duration_s,
    fingerprint_id: "3F2A-91C7",
    in_use: used,
  });
  const savedChecks: SavedCheck[] = [
    ...(cap ? [summary(cap, "main", true)] : []),
    ...(partialCap ? [summary(partialCap, "partial", false)] : []),
  ];
  const view = () => ({
    capability: inUse,
    saved_checks: savedChecks,
    error: null,
    running: opts.checkRunning ?? false,
    fingerprint_changes: opts.fingerprintChange ?? [],
    fingerprint_id: cap ? "3F2A-91C7" : null,
    development: true,
  });
  return (cmd: string, payload?: unknown): unknown => {
    const args = payload as Record<string, unknown> | undefined;
    opts.calls?.push(cmd);
    switch (cmd) {
      case "list_engines":
        return opts.engines === "none" ? NO_ENGINES : REGISTRY;
      case "recheck_registry":
        return undefined;
      case "has_capability_profile":
        if (opts.profile instanceof Error) throw opts.profile;
        return cap !== null || (opts.profile ?? false);
      case "get_capability":
        return view();
      case "run_system_check":
      case "cancel_system_check":
        return undefined;
      case "set_override": {
        const a = args as { kind: OverrideKind; enabled: boolean; confirmed: boolean };
        if (cap) cap.overrides.push({ kind: a.kind, enabled: a.enabled, confirmed: a.confirmed, at: "2026-11-13T11:00:00Z" });
        return view();
      }
      case "session_plan": {
        const a = args as { english: boolean; hindiUrdu: boolean; script: HindiUrduScript };
        return inUse ? planFor(inUse, a.english, a.hindiUrdu, a.script) : null;
      }
      case "list_inputs":
        return INPUTS;
      case "get_settings":
        return SETTINGS;
      case "save_settings":
        return (args as { settings: Settings }).settings;
      case "recorder_snapshot":
        return recording
          ? snapshot(state === "idle" || state === "recovery" ? "recording" : state)
          : snapshot("idle");
      case "recovery_list":
        return { done: true, sessions: state === "recovery" ? [RECOVERED] : [], failed: [] };
      case "subscribe_events": {
        const channel = (args as { onEvent: ChannelLike }).onEvent;
        if (recording) {
          // A level reading, then recorder and mirror status every second, as the app sends them.
          // Channel messages carry an increasing index per channel.
          let next = 0;
          const send = (message: unknown) => emit(channel, [message], next++);
          setTimeout(() => send({ kind: "level", rms_dbfs: -21, peak_dbfs: -11, clipped: false }), 50);
          const status = recorderStatus(state === "input_lost" ? "input_lost" : "recording");
          setInterval(() => {
            send({ kind: "recorder_status", ...status });
            send({ kind: "mirror_status", ...MIRROR });
          }, 1000);
        }
        return null;
      }
      case "start_recording": {
        recording = true;
        const req = (args as { request: { title: string; hall?: string | null } }).request;
        return { ...SESSION, title: req.title, hall: req.hall ?? null, scheduled: null };
      }
      case "recover_session":
        recording = (args as { action: string }).action === "resume";
        return recording ? { ...SESSION, resumed: true } : null;
      case "pause":
      case "resume":
        return null;
      case "stop_session":
        recording = false;
        return {
          recorder: {
            session_dir: DIR,
            parts: PARTS,
            audio_ms: 2_715_000,
            overrun_samples: 0,
            xruns: 0,
            recorder_cpu_ms: 2100,
            manifest_files: 7,
          },
          mirror: { backup_dir: MIRROR.backup_dir, files: [], all_match: true, message: null },
        };
      default:
        throw new Error(`mock bridge: command ${cmd} is not mocked`);
    }
  };
}

const GIB = 1024 ** 3;
const cpuDevice = {
  index: 0,
  backend: "CPU",
  name: "CPU",
  description: "Intel(R) Core(TM) i7-1255U",
  kind: "cpu" as const,
  total_memory_bytes: 16 * GIB,
  free_memory_bytes: 9 * GIB,
  driver_version: null,
  duplicates_dropped: 0,
  shared_memory_bytes: null,
  adapter_name: null,
};
const pin = {
  version: "1.9.5-dev",
  commit: "d1be6fde11ac6e0407606b4e42fe72d34add8037",
  tree_sha256: "e736b4c9a5be2bc0e9b294102e261fe69c2a439976652ff6107b0c361f8667fb",
};

/** What `list_engines` returns in the browser tests: a mix of good and bad rows. */
export const REGISTRY = {
  engines_dir: "C:/Program Files/STELMAR Voice Workbench",
  engines_done: true,
  speech_engine: "available",
  engines: [
    {
      variant: "avx2",
      file: "wb-engine-avx2.exe",
      bytes: 5_300_000,
      state: "ok",
      message: null,
      devices: [cpuDevice],
      whisper_cpp: pin,
      backends: ["CPU"],
      info_ms: 180,
    },
    {
      variant: "avx512",
      file: "wb-engine-avx512.exe",
      bytes: 5_500_000,
      state: "cpu_mismatch",
      message:
        "This engine build (avx512) needs CPU features this computer does not have: avx512f, avx512cd, avx512vl, avx512dq, avx512bw.",
      devices: [],
      whisper_cpp: pin,
      backends: [],
      info_ms: 40,
    },
    {
      variant: "vulkan",
      file: "wb-engine-vulkan.exe",
      bytes: 49_400_000,
      state: "ok",
      message: null,
      devices: [
        cpuDevice,
        {
          ...cpuDevice,
          index: 1,
          backend: "Vulkan",
          name: "Vulkan0",
          description: "Intel(R) Iris(R) Xe Graphics",
          kind: "igpu" as const,
          total_memory_bytes: 8 * GIB,
          driver_version: "31.0.101.4577",
          shared_memory_bytes: 8 * GIB,
        },
      ],
      whisper_cpp: pin,
      backends: ["CPU", "Vulkan"],
      info_ms: 910,
    },
  ],
  models_dir: "C:/Program Files/STELMAR Voice Workbench/models",
  models_done: true,
  manifest_error: null,
  models: [
    {
      id: "small",
      file: "ggml-small.bin",
      bytes: 487_601_967,
      base: "small",
      quant: "f16",
      languages: ["en"],
      candidate_roles: ["near-live"],
      validated_roles: [],
      licence: "MIT",
      state: "ok",
      message: null,
    },
    {
      id: "medium",
      file: "ggml-medium.bin",
      bytes: 1_533_763_059,
      base: "medium",
      quant: "f16",
      languages: ["en", "hi", "ur"],
      candidate_roles: ["rolling"],
      validated_roles: [],
      licence: "MIT",
      state: "ok",
      message: null,
    },
    {
      id: "hindi-medium-q8_0",
      file: "ggml-whisper-hindi-medium-q8_0.bin",
      bytes: 823_000_000,
      base: "medium",
      quant: "q8_0",
      languages: ["hi"],
      candidate_roles: ["rolling", "night"],
      validated_roles: [],
      licence: "Apache-2.0",
      state: "hash_mismatch",
      message: "checksum does not match",
    },
    {
      id: "large-v3-q5_0",
      file: "ggml-large-v3-q5_0.bin",
      bytes: 1_080_000_000,
      base: "large-v3",
      quant: "q5_0",
      languages: ["en", "hi", "ur"],
      candidate_roles: ["night"],
      validated_roles: [],
      licence: "MIT",
      state: "missing",
      message: "file not found",
    },
  ],
};

/** What `list_engines` returns with every speech engine renamed away (models are unaffected). */
export const NO_ENGINES = {
  ...REGISTRY,
  engines: [],
  speech_engine: "none_found",
};
