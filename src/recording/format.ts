// Formatting for the recording screens: audio-clock times, IST wall-clock times, sizes and the
// meter scale. Pure functions (tested in format.test.ts).

/** Audio-clock milliseconds as HH:MM:SS. */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return [h, m, s].map((n) => String(n).padStart(2, "0")).join(":");
}

/** Audio-clock milliseconds as MM:SS (hours folded into minutes), for part ranges. */
export function minutes(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

/** An RFC 3339 time shown in IST: "08 FEB 2027 · 12:03 IST". */
export function istDateTime(rfc3339: string): string {
  const d = new Date(rfc3339);
  if (Number.isNaN(d.getTime())) return "";
  const parts = new Intl.DateTimeFormat("en-GB", {
    timeZone: "Asia/Kolkata",
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).formatToParts(d);
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
  return `${get("day")} ${get("month").toUpperCase()} ${get("year")} · ${get("hour")}:${get("minute")} IST`;
}

/** Bytes as "388 GB" (decimal, as Windows Explorer's drive bar does not; operators read GB). */
export function gigabytes(bytes: number): string {
  return `${Math.round(bytes / 1e9)} GB`;
}

/** Sample rate and channels: "48 kHz · 2 ch". */
export function format(rate: number, channels: number): string {
  const khz = rate % 1000 === 0 ? String(rate / 1000) : (rate / 1000).toFixed(1);
  return `${khz} kHz · ${channels} ch`;
}

/** Meter position (0 to 100) for a dBFS value on the −60 to 0 scale. */
export function meterPercent(dbfs: number): number {
  if (!Number.isFinite(dbfs)) return 0;
  return Math.min(100, Math.max(0, ((dbfs + 60) / 60) * 100));
}

/** Language codes as the badge shows them: ["en", "hi"] -> "EN · HI". */
export function languages(codes: string[]): string {
  return codes.map((c) => c.toUpperCase()).join(" · ");
}

/** File name of part n: part-0001.wav. */
export function partFile(n: number): string {
  return `part-${String(n).padStart(4, "0")}.wav`;
}
