// What the System screen and the Start Session dialog say, from a capability profile. Pure functions
// (tested in view.test.ts). All speeds are real-time factors; the screen shows them as "0.73×".
import type {
  CapGpu,
  CapLang,
  CapMeasurement,
  Capability,
  DeviceConfig,
  OverrideKind,
  SessionPlan,
} from "../ipc";

/** Shown wherever transcription falls behind the recording (owner, 13 Nov 2026). */
export const LAG_TEXT = "transcripts may take hours on this computer; the recording is not affected";

export const LANG_NAME: Record<CapLang, string> = { en: "English", hi: "Hindi", ur: "Urdu" };

export function sameConfig(a: DeviceConfig, b: DeviceConfig): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "gpu" && b.kind === "gpu") return a.index === b.index;
  if (a.kind === "hybrid" && b.kind === "hybrid") return a.gpu === b.gpu;
  return true;
}

export function gib(bytes: number): string {
  const v = bytes / 2 ** 30;
  return `${v >= 10 ? Math.round(v) : v.toFixed(1)} GB`;
}

export function times(rtf: number): string {
  return `${rtf.toFixed(2)}×`;
}

/** Minutes as "about 7 min" or "about 1 h 7 min". */
export function aboutMinutes(min: number): string {
  const m = Math.max(1, Math.round(min));
  if (m < 60) return `about ${m} min`;
  const h = Math.floor(m / 60);
  const r = m % 60;
  return r === 0 ? `about ${h} h` : `about ${h} h ${r} min`;
}

export function checkTime(seconds: number): string {
  const s = Math.round(seconds);
  return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
}

function gpuOf(cap: Capability, index: number): CapGpu | undefined {
  return cap.hardware.gpus.find((g) => g.index === index);
}

/** "cpu 3", "uhd graphics", "cpu 1 + uhd graphics": the device configuration of a row. */
export function configLabel(cap: Capability, m: Pick<CapMeasurement, "config" | "threads">): string {
  const gpu = (i: number) => (gpuOf(cap, i)?.name ?? `gpu ${i}`).replace(/\(R\)|\(TM\)/g, "").trim().toLowerCase();
  switch (m.config.kind) {
    case "cpu":
      return `cpu ${m.threads}`;
    case "gpu":
      return gpu(m.config.index);
    case "hybrid":
      return `cpu ${m.threads} + ${gpu(m.config.gpu)}`;
  }
}

export interface SpeedRow {
  model: string;
  role: string;
  devices: string;
  speed: string;
  memory: string;
  result: { label: string; tone: "success" | "warning" | "info" | "neutral" } | null;
}

function roleText(m: CapMeasurement): string {
  const lang = LANG_NAME[m.lang];
  if (m.live_standin) return `Live test, ${lang}`;
  return `${m.mode === "near_live" ? "Near-live" : "Rolling"}, ${lang}`;
}

/** The Speed test table: one row per measurement, with what the rules decided for it. */
export function speedRows(cap: Capability): SpeedRow[] {
  const d = cap.decision;
  const chosen = d.config?.config;
  const bars = d.bars;
  return cap.measurements.map((m) => {
    const lang = d.languages[m.lang];
    const onChosen =
      chosen && (sameConfig(chosen, m.config) || (chosen.kind === "hybrid" && m.config.kind === "gpu" && m.config.index === chosen.gpu));
    let result: SpeedRow["result"] = null;
    if (m.live_standin) {
      result = m.rtf <= bars.live_standin_rtf ? { label: "Fits live", tone: "success" } : { label: "Too slow", tone: "warning" };
    } else if (m.mode === "near_live") {
      result =
        m.rtf >= 1
          ? { label: "Slower than real time", tone: "warning" }
          : m.rtf > bars.near_live_rtf || m.truncated_chunks > 0
            ? { label: "Too slow", tone: "warning" }
            : { label: "Fits", tone: "info" };
    } else if (lang?.rolling?.model === m.model && onChosen) {
      result = { label: "Chosen", tone: "success" };
    } else if (lang?.night?.model === m.model && onChosen) {
      result = { label: "Fits overnight", tone: "info" };
    } else if (m.rtf > bars.rolling_rtf) {
      result = { label: "Too slow", tone: "warning" };
    }
    return {
      model: m.model,
      role: roleText(m),
      devices: configLabel(cap, m),
      speed: times(m.rtf),
      memory: gib(m.peak_commit_bytes + (m.gpu_local_bytes ?? 0)),
      result,
    };
  });
}

/** "Core i5-1235U · Iris Xe": the processor and the first graphics adapter, for the top bar. */
export function machineLabel(cap: Capability): string {
  const clean = (t: string) =>
    t
      .replace(/\((R|TM)\)/g, "")
      .replace(/^Intel\s+/i, "")
      .replace(/^NVIDIA\s+(GeForce\s+)?/i, "")
      .replace(/\s+(CPU|Processor)\b.*$/i, "")
      .replace(/\s+Graphics$/i, "")
      .replace(/\s+/g, " ")
      .trim();
  const gpu = cap.hardware.gpus.find((g) => g.vulkan_usable);
  const cpu = clean(cap.hardware.cpu_brand);
  return gpu ? `${cpu} · ${clean(gpu.name)}` : cpu;
}

export interface HardwareRow {
  part: string;
  found: string;
  mono?: boolean;
  result: { label: string; tone: "success" | "warning" | "neutral" };
}

export function hardwareRows(cap: Capability): HardwareRow[] {
  const hw = cap.hardware;
  const d = cap.decision;
  const cpuOk = hw.avx2;
  const gpus = hw.gpus.filter((g) => g.vulkan_usable);
  const hybrid = d.config?.config.kind === "hybrid";
  const power = hw.on_mains_power === null ? "power source unknown" : hw.on_mains_power ? "mains power" : "battery power";
  const disks = hw.disks.map((x) => `${x.path.slice(0, 2)} ${Math.round(x.free_bytes / 1e9)} GB free`).join(" · ");
  const memOk = hw.total_ram_bytes >= 8 * 2 ** 30;
  return [
    {
      part: "Processor",
      found: `${hw.cpu_brand} · ${hw.physical_cores} cores, ${hw.logical_processors} threads · AVX2 ${hw.avx2 ? "yes" : "no"}, AVX-512 ${hw.avx512 ? "yes" : "no"}`,
      result: cpuOk ? { label: "OK", tone: "success" } : { label: "Not supported", tone: "warning" },
    },
    {
      part: "Memory",
      found: `${gib(hw.total_ram_bytes)} · ${gib(hw.available_ram_bytes)} free`,
      mono: true,
      result: memOk ? { label: "OK", tone: "success" } : { label: "Too little", tone: "warning" },
    },
    {
      part: "Graphics",
      found:
        gpus.length === 0
          ? "No graphics adapter the speech engine can use"
          : gpus
              .map((g) => `${g.name} (Vulkan)${g.kind === "integrated" ? " · shared memory" : ` · ${gib(g.dedicated_bytes)}`}`)
              .join(" · ") + (hybrid ? " · faster together with the CPU" : ""),
      result:
        gpus.length === 0
          ? { label: "CPU only", tone: "neutral" }
          : hybrid
            ? { label: "Hybrid", tone: "success" }
            : { label: "OK", tone: "success" },
    },
    {
      part: "Storage and power",
      found: `${disks || "free space unknown"} · ${power}`,
      result: { label: "OK", tone: "success" },
    },
    {
      part: "Audio input",
      found: hw.audio_inputs === 0 ? "No audio input found" : `${hw.audio_inputs} input${hw.audio_inputs === 1 ? "" : "s"}`,
      result: hw.audio_inputs === 0 ? { label: "None", tone: "warning" } : { label: "OK", tone: "success" },
    },
  ];
}

export interface ReadyRow {
  label: string;
  value: string;
  bold?: boolean;
}

/** The Verdict card's paragraphs: the rules' summary lines without the per-language lines, which the
 * "Ready after a 90-minute talk" card and the speed table already show. */
export function verdictLines(cap: Capability): string[] {
  return cap.decision.verdict.filter((line) => !/^(English|Hindi|Urdu)\b/.test(line));
}

/** The "Ready after a 90-minute talk" card: English, Hindi, Urdu and the night pass. */
export function readyRows(cap: Capability): ReadyRow[] {
  const rows: ReadyRow[] = [];
  for (const lang of ["en", "hi", "ur"] as const) {
    const r = cap.decision.languages[lang]?.rolling;
    if (!r) {
      rows.push({ label: LANG_NAME[lang], value: "not measured" });
    } else if (r.lags) {
      rows.push({ label: LANG_NAME[lang], value: `${aboutMinutes(r.ready_after_stop_min)}; ${LAG_TEXT}` });
    } else {
      rows.push({ label: LANG_NAME[lang], value: aboutMinutes(r.ready_after_stop_min) });
    }
  }
  const nightLangs = (["en", "hi", "ur"] as const).filter((l) => !!cap.decision.languages[l]?.night);
  const measured = (["en", "hi", "ur"] as const).every((l) => cap.decision.languages[l]?.night_note !== "The night model is measured by the full check.");
  rows.push({
    label: "Night pass",
    value:
      nightLangs.length > 0
        ? `Done by 08:00 (${nightLangs.map((l) => LANG_NAME[l]).join(", ")})`
        : measured
          ? "None needed"
          : "Measured by the full check",
  });
  return rows;
}

export const OVERRIDE_TEXT: Record<OverrideKind, { label: string; risk: string }> = {
  process_while_recording: {
    label: "Process while recording",
    risk: "Transcription runs in the background while recording continues. Turning it off saves the processor for the recorder, and each talk is transcribed after Stop instead.",
  },
  pause_on_battery: {
    label: "Pause on battery power",
    risk: "Background transcription pauses when the computer runs on its battery. Turning it off lets it use the battery, which can run it flat during a session.",
  },
  allow_live_anyway: {
    label: "Allow live captions anyway",
    risk: "Live captions are offered although the system check measured this computer as too slow for them. Captions can lag, drop words or load the processor while recording. Recording itself stays protected.",
  },
};

/** Current state of an override: the last logged change, else the verdict's default. */
export function overrideOn(cap: Capability, kind: OverrideKind): boolean {
  const last = [...cap.overrides].reverse().find((o) => o.kind === kind);
  if (last) return last.enabled && last.confirmed;
  if (kind === "process_while_recording") return cap.decision.process_while_recording;
  if (kind === "pause_on_battery") return cap.decision.pause_on_battery;
  return false;
}

/** Start Session dialog: the mode card text for post-processing. */
export function recordPostText(plan: SessionPlan | null): string {
  if (!plan) return "Transcribed on this machine while recording continues.";
  if (plan.record_only) {
    return "This machine records only; transcription runs on another computer.";
  }
  if (plan.ready_after_stop_min === null) return "Transcribed on this machine while recording continues.";
  return plan.lags
    ? `Transcribed on this machine, but it falls behind the recording: ready ${aboutMinutes(plan.ready_after_stop_min)} after Stop for a 90-minute talk. Transcripts may take hours on this computer; the recording is not affected.`
    : `Transcribed on this machine while recording continues. Ready ${aboutMinutes(plan.ready_after_stop_min)} after Stop.`;
}
