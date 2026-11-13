import { describe, expect, it } from "vitest";
import { profile, planFor } from "../test/profiles";
import {
  aboutMinutes,
  checkTime,
  gib,
  hardwareRows,
  overrideOn,
  readyRows,
  recordPostText,
  speedRows,
  times,
} from "./view";

describe("formatting", () => {
  it("writes speeds, sizes and times the way the mockup does", () => {
    expect(times(0.734)).toBe("0.73×");
    expect(gib(1.4 * 2 ** 30)).toBe("1.4 GB");
    expect(gib(16 * 2 ** 30)).toBe("16 GB");
    expect(aboutMinutes(7.3)).toBe("about 7 min");
    expect(aboutMinutes(66.6)).toBe("about 1 h 7 min");
    expect(aboutMinutes(120)).toBe("about 2 h");
    expect(checkTime(312)).toBe("5 min 12 s");
  });
});

describe("the speed test table", () => {
  it("marks the chosen rows and the rows that miss the bar", () => {
    const cap = profile("tier-c-low");
    const rows = speedRows(cap);
    expect(rows.length).toBe(cap.measurements.length);
    expect(rows.some((r) => r.result?.label === "Chosen")).toBe(true);
    expect(rows.filter((r) => r.role.startsWith("Near-live")).every((r) => r.result?.tone === "warning")).toBe(true);
  });

  it("names the device configuration of each row", () => {
    const rows = speedRows(profile("tier-c-mid-hybrid"));
    expect(rows.some((r) => /^cpu \d+$/.test(r.devices))).toBe(true);
    expect(rows.some((r) => /graphics/.test(r.devices))).toBe(true);
  });
});

describe("hardware and ready time", () => {
  it("has the five parts of the mockup in order", () => {
    expect(hardwareRows(profile("tier-g")).map((r) => r.part)).toEqual([
      "Processor",
      "Memory",
      "Graphics",
      "Storage and power",
      "Audio input",
    ]);
  });

  it("reports a machine with no usable graphics as CPU only", () => {
    const graphics = hardwareRows(profile("tier-c-cpu-only"))[2];
    expect(graphics?.result.label).toBe("CPU only");
  });

  it("says a lagging language lags", () => {
    const rows = readyRows(profile("tier-c-cpu-only"));
    expect(rows.find((r) => r.label === "Hindi")?.value).toMatch(/take hours on this computer; the recording is not affected/);
    expect(rows.find((r) => r.label === "English")?.value).not.toMatch(/lags/);
  });
});

describe("overrides", () => {
  it("start from the verdict and follow the last logged change", () => {
    const cap = profile("tier-c-low");
    expect(overrideOn(cap, "allow_live_anyway")).toBe(false);
    expect(overrideOn(cap, "process_while_recording")).toBe(true);
    cap.overrides.push({ kind: "process_while_recording", enabled: false, confirmed: true, at: "2026-11-13T10:00:00Z" });
    expect(overrideOn(cap, "process_while_recording")).toBe(false);
    cap.overrides.push({ kind: "allow_live_anyway", enabled: true, confirmed: false, at: "2026-11-13T10:01:00Z" });
    expect(overrideOn(cap, "allow_live_anyway")).toBe(false);
  });
});

describe("the Start Session plan", () => {
  it("offers live only where every ticked language is cleared", () => {
    const g = profile("tier-g");
    expect(planFor(g, true, true, "devanagari")?.live.selectable).toBe(true);
    const low = profile("tier-c-low");
    const p = planFor(low, true, true, "devanagari");
    expect(p?.live.selectable).toBe(false);
    expect(p?.live.reason).toBeTruthy();
  });

  it("explains a machine that only records", () => {
    const p = planFor(profile("tier-r"), true, true, "devanagari");
    expect(p?.record_only).toBe(true);
    expect(p?.live.selectable).toBe(false);
    expect(recordPostText(p)).toMatch(/records only/);
  });

  it("states the lag for a machine that falls behind", () => {
    const p = planFor(profile("tier-c-cpu-only"), false, true, "devanagari");
    expect(p?.lags).toBe(true);
    expect(recordPostText(p)).toMatch(/falls behind/);
  });

  it("follows the script choice from Hindi to Urdu", () => {
    const p = planFor(profile("tier-c-low"), false, true, "nastaliq");
    expect(p?.models.map((m) => m.lang)).toEqual(["ur"]);
  });
});
