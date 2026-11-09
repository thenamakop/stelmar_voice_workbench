// The recorder as the UI sees it: the snapshot from the app (refetched on every load, so a
// display that was recreated after a WebView2 failure picks up where it was), the event channel,
// the input level with a peak hold, crash recovery and the recording commands.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  getSettings,
  listInputs,
  pauseRecording,
  recorderSnapshot,
  recoverSession,
  recoveryList,
  resumeRecording,
  saveSettings as saveSettingsCmd,
  startSession,
  stopSession,
  subscribeEvents,
} from "../ipc";
import type {
  AppEvent,
  InputDevice,
  RecoveredSession,
  SessionStopped,
  Settings,
  Snapshot,
  StartRequest,
} from "../ipc";
import type { ToastTone } from "../ds";

export interface Notify {
  (toast: { tone: ToastTone; title: string; message?: string }): void;
}

export interface Level {
  rms: number;
  peak: number;
  /** Peak held for 1.5 s, then falling at 20 dB a second. */
  hold: number;
  clipped: boolean;
}

export interface Recorder {
  snapshot: Snapshot | null;
  level: Level | null;
  inputs: InputDevice[];
  settings: Settings | null;
  recovery: RecoveredSession[];
  busy: boolean;
  /** No mirror status for 5 seconds during a session (backup drive hung). */
  mirrorStalled: boolean;
  start: (request: StartRequest) => Promise<boolean>;
  pause: () => Promise<void>;
  resume: () => Promise<void>;
  stop: () => Promise<SessionStopped | null>;
  recover: (dir: string, action: "resume" | "close") => Promise<void>;
  refreshInputs: () => Promise<void>;
  saveSettings: (settings: Settings) => Promise<boolean>;
}

const EMPTY: Snapshot = {
  session: null,
  recorder: null,
  mirror: null,
  parts: [],
  last_stop: null,
  display_restarts: 0,
  display_warning: null,
};

const HOLD_MS = 1500;
const FALL_DB_PER_EVENT = 1; // 20 events a second -> 20 dB/s

/** The event's payload without its `kind` tag. */
function withoutKind<T extends { kind: string }>(e: T): Omit<T, "kind"> {
  const copy: Partial<T> = { ...e };
  delete copy.kind;
  return copy as Omit<T, "kind">;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function useRecorder(notify: Notify): Recorder {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [level, setLevel] = useState<Level | null>(null);
  const [inputs, setInputs] = useState<InputDevice[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [recovery, setRecovery] = useState<RecoveredSession[]>([]);
  const [busy, setBusy] = useState(false);
  const holdAt = useRef(0);
  const [mirrorStalled, setMirrorStalled] = useState(false);
  const lastRecorderAt = useRef(0);
  const lastMirrorAt = useRef(0);
  const recorderStaleWarned = useRef(false);
  const notifyRef = useRef(notify);
  useEffect(() => {
    notifyRef.current = notify;
  }, [notify]);

  const onEvent = useCallback((e: AppEvent) => {
    switch (e.kind) {
      case "level": {
        const now = performance.now();
        setLevel((prev) => {
          let hold = prev ? prev.hold : -100;
          if (e.peak_dbfs >= hold) {
            hold = e.peak_dbfs;
            holdAt.current = now;
          } else if (now - holdAt.current > HOLD_MS) {
            hold = Math.max(e.peak_dbfs, hold - FALL_DB_PER_EVENT);
          }
          return { rms: e.rms_dbfs, peak: e.peak_dbfs, hold, clipped: e.clipped };
        });
        break;
      }
      case "recorder_status": {
        const status = withoutKind(e);
        lastRecorderAt.current = performance.now();
        recorderStaleWarned.current = false;
        setSnapshot((s) => ({ ...(s ?? EMPTY), recorder: status }));
        if (status.state !== "recording") setLevel(null);
        break;
      }
      case "mirror_status": {
        const status = withoutKind(e);
        lastMirrorAt.current = performance.now();
        setMirrorStalled(false);
        setSnapshot((s) => ({ ...(s ?? EMPTY), mirror: status }));
        break;
      }
      case "part_closed": {
        const part = withoutKind(e);
        setSnapshot((s) => {
          const base = s ?? EMPTY;
          if (base.parts.some((p) => p.part === part.part && p.start_ms === part.start_ms)) return base;
          return { ...base, parts: [...base.parts, part] };
        });
        break;
      }
      case "notice":
        if (e.level !== "info") {
          notifyRef.current({
            tone: e.level === "error" ? "danger" : "warning",
            title: e.source === "mirror" ? "Backup" : e.source === "capture" ? "Audio input" : "Recorder",
            message: e.message,
          });
        }
        break;
      case "close_blocked":
        notifyRef.current({
          tone: "warning",
          title: "A session is recording",
          message: "Stop the session before closing the app.",
        });
        break;
      case "display_recovered":
        break;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    (async () => {
      try {
        await subscribeEvents(onEvent);
        const [snap, devices, cfg] = await Promise.all([recorderSnapshot(), listInputs(), getSettings()]);
        if (cancelled) return;
        setSnapshot(snap);
        setInputs(devices);
        setSettings(cfg);
        if (snap.display_warning) {
          notifyRef.current({ tone: "warning", title: "Display restarted", message: snap.display_warning });
        }
      } catch (err) {
        if (!cancelled) {
          setSnapshot(EMPTY);
          notifyRef.current({ tone: "danger", title: "Could not reach the recorder", message: message(err) });
        }
      }
      // Crash recovery: the scan repairs files at start-up; poll until it is done.
      const poll = async () => {
        try {
          const list = await recoveryList();
          if (cancelled) return;
          setRecovery(list.sessions);
          if (list.done && list.failed.length > 0) {
            notifyRef.current({
              tone: "danger",
              title: "A session could not be recovered",
              message: `${list.failed.join("; ")}. Its audio files are untouched; ask for help before deleting anything.`,
            });
          }
          if (!list.done) timer = setTimeout(poll, 1000);
        } catch {
          /* recovery is offered again on the next start */
        }
      };
      void poll();
    })();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [onEvent]);

  // Watchdog: during a session the recorder reports 4 times a second and the mirror once a
  // second. Silence means a stuck thread or a hung backup drive; say so.
  const sessionOpen = !!snapshot?.session;
  useEffect(() => {
    if (!sessionOpen) return;
    const started = performance.now();
    lastRecorderAt.current = Math.max(lastRecorderAt.current, started);
    lastMirrorAt.current = Math.max(lastMirrorAt.current, started);
    const id = setInterval(() => {
      const now = performance.now();
      if (now - lastRecorderAt.current > 3000 && !recorderStaleWarned.current) {
        recorderStaleWarned.current = true;
        notifyRef.current({
          tone: "danger",
          title: "The recorder is not responding",
          message:
            "No status for 3 seconds. Check the audio input; if this stays, stop and resume the session.",
        });
      }
      setMirrorStalled(now - lastMirrorAt.current > 5000);
    }, 1000);
    return () => clearInterval(id);
  }, [sessionOpen]);

  const run = useCallback(async <T>(title: string, fn: () => Promise<T>): Promise<T | null> => {
    setBusy(true);
    try {
      return await fn();
    } catch (err) {
      notifyRef.current({ tone: "danger", title, message: message(err) });
      return null;
    } finally {
      setBusy(false);
    }
  }, []);

  const start = useCallback(
    async (request: StartRequest) => {
      const info = await run("Could not start the session", () => startSession(request));
      if (!info) return false;
      setLevel(null);
      setSnapshot((s) => ({
        ...(s ?? EMPTY),
        session: info,
        parts: [],
        last_stop: null,
        recorder: null,
        mirror: null,
      }));
      return true;
    },
    [run],
  );

  const stop = useCallback(async () => {
    const result = await run("Could not stop the session", () => stopSession());
    if (!result) {
      // A failed Stop keeps the session and offers it for Resume or Close.
      const [snap, list] = await Promise.all([
        recorderSnapshot().catch(() => null),
        recoveryList().catch(() => null),
      ]);
      if (snap) setSnapshot(snap);
      if (list) setRecovery(list.sessions);
    }
    if (result) {
      setSnapshot((s) => ({ ...(s ?? EMPTY), session: null, last_stop: result }));
      setLevel(null);
      const mirror = result.mirror.backup_dir
        ? result.mirror.all_match
          ? "Backup copy verified."
          : "The backup copy does not match; check the backup drive."
        : "No backup folder is set.";
      notifyRef.current({
        tone: result.mirror.backup_dir && !result.mirror.all_match ? "warning" : "success",
        title: "Session stopped",
        message: `${result.recorder.parts.length} part${result.recorder.parts.length === 1 ? "" : "s"} saved. ${mirror}`,
      });
    }
    return result;
  }, [run]);

  const pause = useCallback(async () => {
    await run("Could not pause", () => pauseRecording());
  }, [run]);

  const resume = useCallback(async () => {
    await run("Could not resume", () => resumeRecording());
  }, [run]);

  const recover = useCallback(
    async (dir: string, action: "resume" | "close") => {
      const info = await run(
        action === "resume" ? "Could not resume the session" : "Could not close the session",
        () => recoverSession(dir, action),
      );
      setRecovery((list) => (info !== null || action === "close" ? list.filter((r) => r.dir !== dir) : list));
      if (info) {
        const snap = await recorderSnapshot().catch(() => null);
        setSnapshot((s) => ({ ...(snap ?? s ?? EMPTY), session: info }));
      }
    },
    [run],
  );

  const refreshInputs = useCallback(async () => {
    const devices = await run("Could not list audio inputs", () => listInputs());
    if (devices) setInputs(devices);
  }, [run]);

  const saveSettings = useCallback(
    async (next: Settings) => {
      const saved = await run("Could not save the settings", () => saveSettingsCmd(next));
      if (saved) setSettings(saved);
      return !!saved;
    },
    [run],
  );

  return {
    snapshot,
    level,
    inputs,
    settings,
    recovery,
    busy,
    mirrorStalled,
    start,
    pause,
    resume,
    stop,
    recover,
    refreshInputs,
    saveSettings,
  };
}
