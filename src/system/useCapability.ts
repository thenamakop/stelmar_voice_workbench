// The saved capability profile and the system check, shared by the System screen, the Start Session
// dialog and the top bar. The profile is read from the app (capability.json); the check runs in the app
// process and reports progress as events.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelSystemCheck,
  getCapability,
  runSystemCheck,
  setOverride as setOverrideCmd,
} from "../ipc";
import type { AppEvent, CapabilityView, CheckProgress, OverrideKind } from "../ipc";
import type { Notify } from "../recording/useRecorder";

export interface CapabilityState {
  view: CapabilityView | null;
  running: boolean;
  progress: CheckProgress | null;
  run: (full: boolean) => Promise<void>;
  cancel: () => Promise<void>;
  setOverride: (kind: OverrideKind, enabled: boolean, confirmed: boolean) => Promise<boolean>;
  refresh: () => Promise<void>;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function useCapability(
  subscribeCheck: (listener: (event: AppEvent) => void) => () => void,
  notify: Notify,
): CapabilityState {
  const [view, setView] = useState<CapabilityView | null>(null);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<CheckProgress | null>(null);
  const notifyRef = useRef(notify);
  useEffect(() => {
    notifyRef.current = notify;
  }, [notify]);

  const refresh = useCallback(async () => {
    try {
      const v = await getCapability();
      setView(v);
      setRunning(v.running);
    } catch (err) {
      notifyRef.current({ tone: "danger", title: "Could not read the system check profile", message: message(err) });
    }
  }, []);

  useEffect(() => {
    let off = false;
    getCapability()
      .then((v) => {
        if (off) return;
        setView(v);
        setRunning(v.running);
      })
      .catch((err: unknown) => {
        if (!off) notifyRef.current({ tone: "danger", title: "Could not read the system check profile", message: message(err) });
      });
    return () => {
      off = true;
    };
  }, []);

  useEffect(
    () =>
      subscribeCheck((e) => {
        if (e.kind === "check_progress") {
          setRunning(true);
          setProgress(e.progress);
        } else if (e.kind === "check_done") {
          setRunning(false);
          setProgress(null);
          void refresh();
          notifyRef.current({
            tone: "success",
            title: "System check finished",
            message: "The profile is saved and used for the next recording.",
          });
        } else if (e.kind === "check_failed") {
          setRunning(false);
          setProgress(null);
          void refresh();
          notifyRef.current({ tone: "danger", title: "The system check did not finish", message: e.message });
        }
      }),
    [subscribeCheck, refresh],
  );

  const run = useCallback(async (full: boolean) => {
    try {
      setRunning(true);
      setProgress({ phase: "Starting", detail: "", elapsed_s: 0, cells_done: 0 });
      await runSystemCheck(full);
    } catch (err) {
      setRunning(false);
      setProgress(null);
      notifyRef.current({ tone: "danger", title: "Could not start the system check", message: message(err) });
    }
  }, []);

  const cancel = useCallback(async () => {
    try {
      await cancelSystemCheck();
    } catch (err) {
      notifyRef.current({ tone: "danger", title: "Could not cancel the system check", message: message(err) });
    }
  }, []);

  const setOverride = useCallback(async (kind: OverrideKind, enabled: boolean, confirmed: boolean) => {
    try {
      setView(await setOverrideCmd(kind, enabled, confirmed));
      return true;
    } catch (err) {
      notifyRef.current({ tone: "danger", title: "Could not change the override", message: message(err) });
      return false;
    }
  }, []);

  return { view, running, progress, run, cancel, setOverride, refresh };
}
