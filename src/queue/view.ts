// Pure helpers for the Queue screen, the Record screen's transcript rows and the status bar. Nothing here
// touches the bridge, so each rule is tested on its own (view.test.ts).
import type { BadgeTone } from "../ds";
import type { JobKind, JobStateName, PartRecord, QueueJob, QueueSegment, QueueSnapshot } from "../ipc";

export const LANGUAGE_NAMES: Record<string, string> = { en: "English", hi: "Hindi", ur: "Urdu" };

/** The session folder's name as the operator knows it: "2027-02-08/hall__title" or just "hall__title". */
export function sessionLabel(folder: string): string {
  const last = folder.split(/[\\/]/).filter(Boolean).pop() ?? folder;
  const title = last.includes("__") ? last.slice(last.indexOf("__") + 2) : last;
  return title
    .split("-")
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

/** The folder name from a full session path (what the queue commands call a session). */
export function sessionName(dir: string): string {
  return dir.split(/[\\/]/).filter(Boolean).pop() ?? dir;
}

export const KIND_LABEL: Record<JobKind, string> = {
  rolling: "During the day",
  night: "Night pass",
  translate: "Translation",
  import: "Imported",
};

export const STATE_BADGE: Record<JobStateName, [BadgeTone, string]> = {
  queued: ["neutral", "Waiting"],
  running: ["info", "Running"],
  paused: ["warning", "Paused"],
  done: ["success", "Done"],
  parked: ["danger", "Needs attention"],
};

/** "English: small · Hindi: turbo-q5_0", in the language order of the job. */
export function modelsText(job: QueueJob): string {
  const parts = Object.entries(job.models_by_lang).map(
    ([lang, id]) => `${LANGUAGE_NAMES[lang] ?? lang}: ${id}`,
  );
  return parts.length > 0 ? parts.join(" · ") : "No model chosen";
}

/** How much of a job is committed, 0 to 1 (a job with no audio yet has none). */
export function jobProgress(job: QueueJob): number {
  if (job.state === "done") return 1;
  const total = job.cursor_ms + job.remaining_audio_ms;
  return total > 0 ? Math.min(1, job.cursor_ms / total) : 0;
}

export function percent(fraction: number): string {
  return `${Math.round(Math.min(1, Math.max(0, fraction)) * 100)}%`;
}

/** Binary gigabytes with one decimal, as the memory rules count them. */
export function ramText(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

/** Jobs that still have work: waiting, running, paused or parked (a parked job needs the operator). */
export function openJobs(snapshot: QueueSnapshot | null): QueueJob[] {
  return (snapshot?.jobs ?? []).filter((j) => j.state !== "done");
}

/** The status bar's queue text. */
export function statusText(snapshot: QueueSnapshot | null): string {
  if (!snapshot) return "Queue: no jobs";
  const open = openJobs(snapshot);
  if (open.length === 0) return "Queue: no jobs";
  const running = open.filter((j) => j.state === "running").length;
  const parked = open.filter((j) => j.state === "parked").length;
  const bits = [`${open.length} job${open.length === 1 ? "" : "s"}`];
  if (running > 0) bits.push(`${running} running`);
  if (parked > 0) bits.push(`${parked} need${parked === 1 ? "s" : ""} attention`);
  if (snapshot.scheduler.paused) bits.push("paused");
  return `Queue: ${bits.join(" · ")}`;
}

/** The job that decodes a session during the day (the one whose cursor the parts strip follows). */
export function rollingJob(snapshot: QueueSnapshot | null, session: string | null): QueueJob | null {
  if (!snapshot || !session) return null;
  return (
    snapshot.jobs.find((j) => j.session === session && (j.kind === "rolling" || j.kind === "import")) ?? null
  );
}

export type PartProgress = { tone: "done" } | { tone: "run"; fraction: number } | { tone: "wait" };

/**
 * Where one closed part stands in its session's rolling job. `cursor_ms` is the audio time up to which every
 * segment is committed, so a part is done when the cursor has passed its end; the first part the cursor has not
 * passed is "run" while the job runs and some of it is committed or an engine is working; the rest wait.
 */
export function partProgress(part: PartRecord, job: QueueJob | null, firstOpen: boolean): PartProgress {
  if (!job) return { tone: "wait" };
  const end = part.start_ms + part.duration_ms;
  if (job.cursor_ms >= end) return { tone: "done" };
  if (firstOpen && job.state === "running") {
    const fraction = Math.min(
      1,
      Math.max(0, (job.cursor_ms - part.start_ms) / Math.max(1, part.duration_ms)),
    );
    return { tone: "run", fraction };
  }
  return { tone: "wait" };
}

/** Badge text for a segment's language: HI·EN when the segment holds both. */
export function languageBadge(seg: Pick<QueueSegment, "lang" | "mixed">): string {
  if (seg.mixed) return "HI·EN";
  return seg.lang.toUpperCase();
}

/** The font and direction a segment is shown in. */
export type RowKind = "en" | "hi" | "ur" | "mixed";

export function rowKind(seg: Pick<QueueSegment, "lang" | "mixed">): RowKind {
  if (seg.mixed) return "mixed";
  if (seg.lang === "hi" || seg.lang === "ur") return seg.lang;
  return "en";
}

/** Adds committed segments to the shown ones: ordered by start, no second row for the same start. */
export function mergeSegments(shown: QueueSegment[], added: QueueSegment[]): QueueSegment[] {
  const byStart = new Map<number, QueueSegment>();
  for (const s of shown) byStart.set(s.start_ms, s);
  for (const s of added) byStart.set(s.start_ms, s);
  return [...byStart.values()].sort((a, b) => a.start_ms - b.start_ms);
}

/** Minutes of audio the queue is behind: what has been recorded but not yet committed. */
export function behindMs(recordedMs: number, job: QueueJob | null): number {
  if (!job) return 0;
  return Math.max(0, recordedMs - job.cursor_ms);
}

export function behindText(ms: number): string {
  const min = Math.round(ms / 60000);
  return min < 1 ? "Up to date" : `${min} min`;
}
