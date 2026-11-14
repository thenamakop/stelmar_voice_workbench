// The post-processing queue as the UI sees it: the snapshot from the app (refetched when the app says
// something changed, and every 5 seconds as a fallback), the rolling transcript rows of the sessions the
// operator is looking at, and the commands. Every failure is shown with a Toast, never swallowed.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  importSession,
  queueChangeModel,
  queueMoveUp,
  queuePause,
  queueResume,
  queueSnapshot,
  sessionTranscript,
  setLanguageLock,
} from "../ipc";
import type { AppEvent, QueueSegment, QueueSnapshot } from "../ipc";
import type { Notify } from "../recording/useRecorder";
import { mergeSegments } from "./view";

const SHOWN_REVISION = "post-1";
const FALLBACK_MS = 5000;

export interface QueueState {
  /** Null until the first answer (and when the app has no queue: no engines or models were found). */
  snapshot: QueueSnapshot | null;
  /** Why the queue is not available, when the app said so. */
  unavailable: string | null;
  /** Committed rows (revision post-1) of one session, in audio order. */
  segmentsFor: (session: string | null) => QueueSegment[];
  /** Starts loading a session's committed rows (those from before this page was opened). */
  watch: (session: string | null) => void;
  pause: (job: number | null) => Promise<void>;
  resume: (job: number | null) => Promise<void>;
  moveUp: (job: number) => Promise<void>;
  changeModel: (job: number, modelsByLang: Record<string, string>) => Promise<boolean>;
  importFolder: (source: string) => Promise<boolean>;
  /** Locks the session to one language from this audio time (null returns to detection); true when saved. */
  lockLanguage: (lang: string | null, audioMs: number) => Promise<boolean>;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function useQueue(
  subscribeQueue: (l: (e: AppEvent) => void) => () => void,
  notify: Notify,
): QueueState {
  const [snapshot, setSnapshot] = useState<QueueSnapshot | null>(null);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [rows, setRows] = useState<Record<string, QueueSegment[]>>({});
  const notifyRef = useRef(notify);
  useEffect(() => {
    notifyRef.current = notify;
  }, [notify]);
  const loaded = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    try {
      const snap = await queueSnapshot();
      setSnapshot(snap);
      setUnavailable(null);
    } catch (err) {
      // No queue (no engines or models found) is a state, not a failure: the Queue screen says it.
      setUnavailable(message(err));
    }
  }, []);

  useEffect(() => {
    const first = setTimeout(() => void refresh(), 0);
    const off = subscribeQueue((e) => {
      if (e.kind === "queue_update" || e.kind === "scheduler_status") void refresh();
      if (e.kind === "segment_committed") {
        const added = e.segments.filter((s) => s.revision === SHOWN_REVISION);
        if (added.length > 0)
          setRows((r) => ({ ...r, [e.session]: mergeSegments(r[e.session] ?? [], added) }));
      }
    });
    const timer = setInterval(() => void refresh(), FALLBACK_MS);
    return () => {
      off();
      clearTimeout(first);
      clearInterval(timer);
    };
  }, [refresh, subscribeQueue]);

  const watch = useCallback((session: string | null) => {
    if (!session || loaded.current.has(session)) return;
    loaded.current.add(session);
    sessionTranscript(session, SHOWN_REVISION)
      .then((segs) => setRows((r) => ({ ...r, [session]: mergeSegments(r[session] ?? [], segs) })))
      .catch(() => {
        // The session may not be in the queue yet (its job is created at the first part): ask again later.
        loaded.current.delete(session);
      });
  }, []);

  /** Runs a command; a failure is shown as a Toast and reported as `ok: false` (a void command answers null). */
  const act = useCallback(
    async <T>(title: string, fn: () => Promise<T>): Promise<{ ok: true; value: T } | { ok: false }> => {
      try {
        const value = await fn();
        void refresh();
        return { ok: true, value };
      } catch (err) {
        notifyRef.current({ tone: "danger", title, message: message(err) });
        return { ok: false };
      }
    },
    [refresh],
  );

  return {
    snapshot,
    unavailable,
    segmentsFor: (session) => (session ? (rows[session] ?? []) : []),
    watch,
    pause: async (job) => void (await act("Could not pause", () => queuePause(job))),
    resume: async (job) => void (await act("Could not resume", () => queueResume(job))),
    moveUp: async (job) => void (await act("Could not move the job up", () => queueMoveUp(job))),
    changeModel: async (job, m) =>
      (await act("Could not change the model", () => queueChangeModel(job, m))).ok,
    lockLanguage: async (lang, audioMs) =>
      (await act("Could not change the language", () => setLanguageLock(lang, audioMs))).ok,
    importFolder: async (source) => {
      const r = await act("Could not import the session", () => importSession(source));
      if (r.ok)
        notifyRef.current({
          tone: "success",
          title: "Session imported",
          message: `${r.value} is queued for transcription.`,
        });
      return r.ok;
    },
  };
}
