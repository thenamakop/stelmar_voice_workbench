// Capability profiles for the mocked bridge: the same JSON files the rules crate's tests check
// (crates/wb-capability/fixtures), so the screens are tested against what the rules produce.
import avx512Crash from "../../crates/wb-capability/fixtures/avx512-crash.json";
import slowIgpu from "../../crates/wb-capability/fixtures/slow-igpu.json";
import tierCCpuOnly from "../../crates/wb-capability/fixtures/tier-c-cpu-only.json";
import tierCLow from "../../crates/wb-capability/fixtures/tier-c-low.json";
import tierCMidHybrid from "../../crates/wb-capability/fixtures/tier-c-mid-hybrid.json";
import tierCPlus from "../../crates/wb-capability/fixtures/tier-c-plus.json";
import tierG from "../../crates/wb-capability/fixtures/tier-g.json";
import tierR from "../../crates/wb-capability/fixtures/tier-r.json";
import type { CapLang, Capability, HindiUrduScript, OverrideKind, SessionPlan } from "../ipc";

export const PROFILES = {
  "tier-g": tierG,
  "tier-c-low": tierCLow,
  "tier-c-mid-hybrid": tierCMidHybrid,
  "tier-c-plus": tierCPlus,
  "tier-c-cpu-only": tierCCpuOnly,
  "tier-r": tierR,
  "slow-igpu": slowIgpu,
  "avx512-crash": avx512Crash,
} as unknown as Record<string, Capability>;

export type ProfileName = keyof typeof PROFILES;

export function profile(name: string): Capability {
  const p = PROFILES[name];
  if (!p) throw new Error(`mock bridge: no capability fixture named ${name}`);
  return structuredClone(p);
}

function overrideOn(cap: Capability, kind: OverrideKind): boolean {
  const last = [...cap.overrides].reverse().find((o) => o.kind === kind);
  return !!last && last.enabled && last.confirmed;
}

/** What `session_plan` answers for a profile (the Rust function is crates/wb-capability session.rs). */
export function planFor(cap: Capability, english: boolean, hindiUrdu: boolean, script: HindiUrduScript): SessionPlan | null {
  const langs: CapLang[] = [...(english ? (["en"] as const) : []), ...(hindiUrdu ? ([script === "devanagari" ? "hi" : "ur"] as const) : [])];
  const d = cap.decision;
  const models = langs.flatMap((lang) => {
    const rp = d.languages[lang]?.rolling;
    return rp ? [{ lang, model: rp.model, quality_unverified: rp.quality_unverified }] : [];
  });
  const picks = langs.map((l) => d.languages[l]?.rolling).filter((x) => !!x);
  const ready = picks.length ? Math.max(...picks.map((p) => p.ready_after_stop_min)) : null;
  const allowAnyway = overrideOn(cap, "allow_live_anyway");
  let live: SessionPlan["live"];
  if (langs.length === 0) {
    live = { selectable: false, reason: "Tick at least one language.", via_override: false };
  } else if (!d.config) {
    live = { selectable: false, reason: d.record_only_reason ?? "This machine records only.", via_override: false };
  } else {
    const blocked = langs
      .map((l) => ({ l, ld: d.languages[l]?.live }))
      .find(({ ld }) => !ld || (!ld.offered && (ld.hard_reasons.length > 0 || !allowAnyway)));
    const soft = langs.some((l) => !d.languages[l]?.live.offered);
    live = blocked
      ? {
          selectable: false,
          reason: blocked.ld?.hard_reasons[0] ?? blocked.ld?.soft_reasons[0] ?? "Live captions are not offered on this machine.",
          via_override: false,
        }
      : { selectable: true, reason: null, via_override: soft };
  }
  return {
    default_mode: "record_postprocess",
    record_only: !d.config,
    record_only_reason: d.record_only_reason,
    live,
    devices: d.config?.label ?? "none",
    models,
    ready_after_stop_min: ready,
    lags: picks.some((p) => p.lags),
    provisional: d.provisional,
    partial: d.partial,
  };
}

/** The profile as the app reads it after a fingerprint change (mirror of `wb_capability::safe_defaults`):
 * live text off for every language with a reason no override can lift, no processing while recording,
 * overrides not applied, the verdict saying why. Recording and post-processing stay as measured. */
export function safeDefaults(cap: Capability, why: string): Capability {
  const c: Capability = structuredClone(cap);
  c.overrides = [];
  c.decision.process_while_recording = false;
  c.decision.provisional = true;
  for (const l of Object.values(c.decision.languages)) {
    if (!l) continue;
    l.live.offered = false;
    l.live.kind = null;
    l.live.model = null;
    l.live.hard_reasons.unshift(why);
  }
  c.decision.verdict.unshift(why);
  return c;
}

