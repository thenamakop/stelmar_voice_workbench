import { describe, expect, it } from "vitest";
import { profile, planFor } from "../test/profiles";
import {
  aboutMinutes,
  checkTime,
  gib,
  hardwareRows,
  machineLabel,
  overrideOn,
  readyRows,
  transcriptionRows,
  liveLanguages,
  joinNames,
  recordPostText,
  speedRows,
  times,
  verdictLines,
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
    expect(rows.find((r) => r.label === "Hindi")?.note).toMatch(/take hours on this computer; the recording is not affected/);
    expect(rows.find((r) => r.label === "Hindi")?.value).toMatch(/^about /);
    expect(rows.find((r) => r.label === "English")?.value).not.toMatch(/lags/);
  });
});

describe("the Record screen's Transcription card", () => {
  it("shows the devices, speed and threads of the saved profile", () => {
    const rows = transcriptionRows(profile("tier-c-low"));
    expect(rows[0]).toEqual(["Devices", "Intel(R) UHD Graphics"]);
    expect(rows[1]?.[0]).toBe("Speed");
    expect(rows[1]?.[1]).toMatch(/^EN\u00a00\.35× · HI\u00a00\.73×/);
    expect(rows[2]).toEqual(["Threads", "2 CPU"]);
  });

  it("names the languages live captions are cleared for", () => {
    expect(liveLanguages(profile("tier-g"))).toEqual(["English"]);
    expect(liveLanguages(profile("tier-c-low"))).toEqual([]);
    expect(joinNames(["English"])).toBe("English");
    expect(joinNames(["English", "Hindi"])).toBe("English and Hindi");
    expect(joinNames(["English", "Hindi", "Urdu"])).toBe("English, Hindi and Urdu");
  });

  it("says not measured without a profile", () => {
    expect(transcriptionRows(null).every(([, v]) => v === "Not measured")).toBe(true);
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
    // Live on the graphics card is English only until Phase 5 checks a Hindi and an Urdu live model.
    const g = profile("tier-g");
    expect(planFor(g, true, false, "devanagari")?.live.selectable).toBe(true);
    const mixed = planFor(g, true, true, "devanagari");
    expect(mixed?.live.selectable).toBe(false);
    expect(mixed?.live.reason).toMatch(/checked for accuracy/);
    expect(planFor(g, false, true, "nastaliq")?.live.selectable).toBe(false);
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

describe("the verdict card", () => {
  it("leaves the per-language lines to the ready-time card", () => {
    const cap = profile("tier-c-low");
    expect(cap.decision.verdict.some((l) => /^English/.test(l))).toBe(true);
    const lines = verdictLines(cap);
    expect(lines.some((l) => /^(English|Hindi|Urdu)/.test(l))).toBe(false);
    expect(lines[0]).toMatch(/^Live text is off by default/);
  });
});

describe("the top bar label", () => {
  it("names the processor and the graphics adapter without the trademark marks", () => {
    const cap = profile("tier-c-low");
    cap.hardware.cpu_brand = "12th Gen Intel(R) Core(TM) i5-1235U";
    expect(machineLabel(cap)).toBe("12th Gen Intel Core i5-1235U · UHD");
    cap.hardware.cpu_brand = "Intel(R) Core(TM) i7-1255U CPU @ 1.70GHz";
    expect(machineLabel(cap)).toMatch(/^Core i7-1255U · /);
  });
});
