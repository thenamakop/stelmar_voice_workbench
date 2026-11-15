import { describe, expect, it } from "vitest";
import type { PartRecord, QueueJob, QueueSegment } from "../ipc";
import { QUEUE_JOBS, SEGMENTS, queueSnapshot } from "../test/mockBridge";
import {
  behindMs,
  behindText,
  deviceLines,
  jobProgress,
  languageBadge,
  mergeSegments,
  modelPairs,
  partProgress,
  rollingJob,
  rowKind,
  sessionLabel,
  sessionName,
  statusText,
} from "./view";

const part = (n: number): PartRecord => ({
  part: n,
  file: `audio/part-000${n}.wav`,
  start_ms: (n - 1) * 600_000,
  duration_ms: 600_000,
  frames: 0,
  sample_rate: 48000,
  channels: 2,
  sha256: "0".repeat(64),
});
const running = QUEUE_JOBS[1] as QueueJob;

describe("names", () => {
  it("turns a session folder into the title the operator knows", () => {
    expect(sessionLabel("seminar-hall-b__parallel-track-sessions-i")).toBe("Parallel Track Sessions I");
    expect(sessionLabel("D:\\rec\\2027-02-08\\hall__inaugural-session")).toBe("Inaugural Session");
    expect(sessionName("D:\\rec\\2027-02-08\\hall__x\\")).toBe("hall__x");
  });
  it("lists the models of a job by language", () => {
    expect(modelPairs(running)).toEqual([
      ["English", "small"],
      ["Hindi", "turbo-q5_0"],
    ]);
  });
});

describe("devices of a job", () => {
  it("says which devices work on a job, with their threads and units", () => {
    expect(deviceLines(running)).toEqual([
      "Graphics · 2 threads · 1 running · 5 done",
      "Processor · 2 threads",
    ]);
    expect(deviceLines({ ...running, devices: [] })).toEqual([]);
    expect(
      deviceLines({ ...running, devices: [{ device: "cpu", threads: 1, units_running: 0, units_committed: 0 }] }),
    ).toEqual(["Processor · 1 thread"]);
  });
});

describe("progress and the status bar", () => {
  it("measures a job by its committed cursor", () => {
    expect(jobProgress(running)).toBeCloseTo(1_500_000 / 2_715_000, 5);
    expect(jobProgress(QUEUE_JOBS[0] as QueueJob)).toBe(1);
    expect(jobProgress({ ...running, cursor_ms: 0, remaining_audio_ms: 0 })).toBe(0);
  });
  it("says what the queue is doing in one line", () => {
    expect(statusText(null)).toBe("Queue: no jobs");
    expect(statusText(queueSnapshot("none"))).toBe("Queue: no jobs");
    expect(statusText(queueSnapshot("busy"))).toBe("Queue: 2 jobs · 1 running");
    expect(statusText(queueSnapshot("paused"))).toBe("Queue: 2 jobs · paused");
    const parked = queueSnapshot("busy");
    parked.jobs = parked.jobs.map((j) => (j.id === 2 ? { ...j, state: "parked" as const } : j));
    expect(statusText(parked)).toBe("Queue: 2 jobs · 1 needs attention");
  });
});

describe("parts of the open session", () => {
  const job = rollingJob(queueSnapshot("busy"), "seminar-hall-b__parallel-track-sessions-i");
  it("finds the session's rolling job", () => {
    expect(job?.id).toBe(2);
    expect(rollingJob(queueSnapshot("busy"), "another")).toBeNull();
    expect(rollingJob(null, "x")).toBeNull();
  });
  it("marks a part done once the cursor passed it, the next one running, the rest waiting", () => {
    expect(partProgress(part(1), job, false)).toEqual({ tone: "done" });
    expect(partProgress(part(2), job, false)).toEqual({ tone: "done" });
    expect(partProgress(part(3), job, true)).toEqual({ tone: "run", fraction: 0.5 });
    expect(partProgress(part(4), job, false)).toEqual({ tone: "wait" });
    expect(partProgress(part(1), null, true)).toEqual({ tone: "wait" });
  });
  it("does not call a part running when the job is held", () => {
    expect(partProgress(part(3), { ...(job as QueueJob), state: "paused" }, true)).toEqual({ tone: "wait" });
  });
  it("reports how far the queue is behind the room", () => {
    expect(behindMs(2_715_000, job)).toBe(1_215_000);
    expect(behindText(1_215_000)).toBe("20 min");
    expect(behindText(20_000)).toBe("Up to date");
    expect(behindMs(2_715_000, null)).toBe(0);
  });
});

describe("rows", () => {
  it("badges each language and marks Hindi with English words HI·EN", () => {
    const [en, hi, mixed, , ur] = SEGMENTS as [
      QueueSegment,
      QueueSegment,
      QueueSegment,
      QueueSegment,
      QueueSegment,
    ];
    expect(languageBadge(en)).toBe("EN");
    expect(languageBadge(hi)).toBe("HI");
    expect(languageBadge(mixed)).toBe("HI·EN");
    expect(languageBadge(ur)).toBe("UR");
    expect([rowKind(en), rowKind(hi), rowKind(mixed), rowKind(ur)]).toEqual(["en", "hi", "mixed", "ur"]);
  });
  it("keeps an Urdu segment with English words in Urdu, not Hindi", () => {
    const seg = { lang: "ur", mixed: true };
    expect(languageBadge(seg)).toBe("UR·EN");
    expect(rowKind(seg)).toBe("mixed-ur");
  });
  it("keeps rows in audio order with one row per start", () => {
    const [a, b, c] = SEGMENTS as [QueueSegment, QueueSegment, QueueSegment];
    const merged = mergeSegments([c, a], [b, { ...a, text: "changed" }]);
    expect(merged.map((s) => s.start_ms)).toEqual([a.start_ms, b.start_ms, c.start_ms]);
    expect(merged[0]?.text).toBe("changed");
  });
});
