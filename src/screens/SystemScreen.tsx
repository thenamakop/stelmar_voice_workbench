import { useState } from "react";
import type { ReactNode } from "react";
import { Badge, Button, Dialog, ProgressBar, Switch } from "../ds";
import { px } from "../ds/units";
import type { Capability, OverrideKind, Tier } from "../ipc";
import { MainWithSide, Note, PageHead, Section, SideCard, Table, KeyValue } from "../shell/layout";
import type { CapabilityState } from "../system/useCapability";
import {
  OVERRIDE_TEXT,
  checkTime,
  hardwareRows,
  joinNames,
  liveLanguages,
  overrideOn,
  readyRows,
  savedCheckRows,
  speedRows,
  verdictLines,
} from "../system/view";
import { EnginesSection, ModelsSection, useRegistry } from "./RegistrySections";
import { liveUnavailableReason } from "./liveReason";

// System screen per design/mockups/system-check.html: the saved capability profile, the numbers behind
// it, and the three overrides. The check itself runs in the app (src-tauri/src/syscheck).
export interface SystemScreenProps {
  capability: CapabilityState;
  /** Leaves the screen for Record (the mockup's Continue button). */
  onContinue?: () => void;
}

/** Overline in IST: "13 NOV 2026 · 16:42 IST". */
function istStamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
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

function savedTime(iso: string): string {
  const s = istStamp(iso);
  return s.slice(s.indexOf("·") + 2);
}

/** The Verdict card's first line on a graphics-card machine: which languages live captions are cleared for. */
function recordModeLine(cap: Capability | null): string {
  const names = liveLanguages(cap);
  if (names.length === 3) return "Record + post-process, live available";
  if (names.length > 0) return `Record + post-process; live for ${joinNames(names)} only`;
  return "Record + post-process";
}

/** The headline: a graphics-card machine says which languages live captions are cleared for. */
function titleFor(cap: Capability, tier: Tier): string {
  if (tier === "G") {
    const names = liveLanguages(cap);
    if (names.length === 3) return TITLE.G;
    if (names.length > 0) return `Record Now, Live Captions for ${joinNames(names)}`;
    return TITLE.C;
  }
  return TITLE[tier];
}

const TITLE: Record<Tier, string> = {
  G: "Record Now, Live Captions Available",
  "C+": "Record Now, Near-Live Text Available",
  C: "Record Now, Transcribe in the Background",
  R: "This Computer Records Only",
};

const TIER_TONE: Record<Tier, "success" | "neutral" | "warning"> = {
  G: "success",
  "C+": "success",
  C: "neutral",
  R: "warning",
};

const OVERRIDES: OverrideKind[] = ["process_while_recording", "pause_on_battery", "allow_live_anyway"];

/** Mono is for times and durations; words such as "not measured" stay in the body face. */
function isDuration(value: string): boolean {
  return /^about \d/.test(value);
}

function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function Notice({
  tone,
  title,
  children,
}: {
  tone: "warning" | "info";
  title: string;
  children?: ReactNode;
}) {
  const fg = tone === "warning" ? "var(--status-warning-fg)" : "var(--status-info-fg)";
  const bg = tone === "warning" ? "var(--status-warning-bg)" : "var(--status-info-bg)";
  return (
    <div
      role="status"
      style={{
        background: bg,
        color: fg,
        borderRadius: "var(--radius-sm)",
        padding: px(10, 14),
        marginBottom: 12,
        fontSize: 13,
        lineHeight: px(20),
      }}
    >
      <b>{title}</b>
      {children && <div style={{ marginTop: 2 }}>{children}</div>}
    </div>
  );
}

export function SystemScreen({ capability, onContinue }: SystemScreenProps) {
  const registry = useRegistry();
  const { view, running, progress } = capability;
  const cap = view?.capability ?? null;
  const [pending, setPending] = useState<{ kind: OverrideKind; enabled: boolean } | null>(null);

  const decision = cap?.decision ?? null;
  const tier = decision?.tier ?? null;
  const blocked = running;

  const toggle = (kind: OverrideKind, enabled: boolean) => {
    // Turning a safeguard off, or live captions on past the verdict, needs a confirmation that is logged.
    const risky = kind === "allow_live_anyway" ? enabled : !enabled;
    if (risky) setPending({ kind, enabled });
    else void capability.setOverride(kind, enabled, true);
  };

  const live = decision ? Object.values(decision.languages).some((l) => l?.live.offered) : false;
  const liveNote = !cap
    ? liveUnavailableReason(false)
    : live
      ? "Live captions are cleared for the languages marked in the speed test."
      : "Live captions are not offered on this computer. The overrides below can allow them anyway, at the operator's risk.";

  return (
    <>
      <PageHead
        overline={cap ? "System check" : "First launch · System check"}
        title={cap && tier ? titleFor(cap, tier) : "Check This Computer Before the First Recording"}
        meta={
          cap ? (
            <>
              <span style={{ fontFamily: "var(--font-mono)" }}>{istStamp(cap.created_at)}</span>
              <span>
                {cap.check.kind === "full" ? "Full" : "Quick"} check took{" "}
                <span style={{ fontFamily: "var(--font-mono)" }}>{checkTime(cap.check.duration_s)}</span>
              </span>
              {tier && <Badge tone={TIER_TONE[tier]}>Tier {tier}</Badge>}
              {cap.emulated && <Badge tone="warning">Emulated · {cap.emulated}</Badge>}
              {cap.check.partial && <Badge tone="warning">Partial</Badge>}
            </>
          ) : (
            <>
              <span>{running ? "Running" : "Not run yet"}</span>
              <Badge>No tier</Badge>
            </>
          )
        }
        controls={
          running ? (
            <Button variant="secondary" onClick={() => void capability.cancel()}>
              Cancel
            </Button>
          ) : (
            <>
              <Button variant="secondary" iconLeft="refresh-cw" onClick={() => void capability.run(!!cap)}>
                {cap ? "Run Full Check" : "Run Quick Check"}
              </Button>
              {cap && onContinue && <Button onClick={onContinue}>Continue</Button>}
            </>
          )
        }
      />
      <MainWithSide
        side={
          <>
            <SideCard title="Verdict">
              <div
                style={{
                  fontSize: 22,
                  lineHeight: px(30),
                  fontWeight: "var(--fw-bold)",
                  letterSpacing: "var(--ls-tight)",
                  color: "var(--ink-900)",
                  margin: px(0, 0, 6),
                }}
              >
                {decision
                  ? decision.tier === "R"
                    ? "Record only"
                    : decision.tier === "G"
                      ? recordModeLine(cap)
                      : "Record + post-process"
                  : "Not checked yet"}
              </div>
              {cap && decision ? (
                verdictLines(cap).map((line, i) => (
                  <Note key={i} style={{ marginTop: i === 0 ? 0 : 6 }}>
                    {line}
                  </Note>
                ))
              ) : (
                <Note>
                  The system check measures this computer and decides how each session is recorded and
                  transcribed here. Live text stays off until the check clears it.
                </Note>
              )}
            </SideCard>
            {cap && (
              <SideCard title="Ready after a 90-minute talk">
                <KeyValue
                  rows={readyRows(cap).map((r): [string, ReactNode, { mono?: boolean }] => [
                    r.label,
                    r.note ? (
                      <>
                        <span style={{ fontFamily: isDuration(r.value) ? "var(--font-mono)" : undefined }}>
                          {r.value}
                        </span>
                        <span
                          style={{
                            display: "block",
                            fontFamily: "var(--font-body)",
                            fontWeight: "var(--fw-regular)",
                            color: "var(--text-muted)",
                          }}
                        >
                          {r.note}
                        </span>
                      </>
                    ) : (
                      r.value
                    ),
                    { mono: isDuration(r.value) && !r.note },
                  ])}
                />
              </SideCard>
            )}
            {view && view.saved_checks.length > 0 && (
              <SideCard title="Saved checks">
                <KeyValue
                  rows={savedCheckRows(view.saved_checks, savedTime).map(
                    (r): [string, ReactNode, { mono?: boolean }] => [
                      r.label,
                      <>
                        <span>{r.value}</span>
                        <span
                          style={{
                            display: "block",
                            fontFamily: "var(--font-body)",
                            fontWeight: "var(--fw-regular)",
                            color: "var(--text-muted)",
                          }}
                        >
                          {r.note}
                        </span>
                      </>,
                      {},
                    ],
                  )}
                />
              </SideCard>
            )}
            <SideCard title="Overrides" stack>
              {OVERRIDES.map((kind) => (
                <Switch
                  key={kind}
                  label={OVERRIDE_TEXT[kind].label}
                  checked={cap ? overrideOn(cap, kind) : kind !== "allow_live_anyway"}
                  disabled={!cap || blocked}
                  onChange={(on) => toggle(kind, on)}
                />
              ))}
              <Note style={{ fontSize: 12, lineHeight: px(18), color: "var(--text-muted)" }}>{liveNote}</Note>
            </SideCard>
          </>
        }
      >
        {running && progress && (
          <div style={{ marginBottom: 14 }}>
            <Notice tone="info" title={`${progress.phase}${progress.detail ? ` · ${progress.detail}` : ""}`}>
              {Math.round(progress.elapsed_s)} s so far · {progress.cells_done} measurements done. Starting a
              recording stops the check and saves what it has measured so far.
            </Notice>
            <ProgressBar
              value={Math.min(0.95, progress.elapsed_s / (cap?.check.bar_s || 480))}
              label="System check"
            />
          </div>
        )}
        {view && view.fingerprint_changes.length > 0 && !running && (
          <Notice tone="warning" title="This computer or the app has changed since the last check">
            {capitalise(view.fingerprint_changes.join(" · "))}. Until the check has run again, this computer
            records and post-processes only: live captions and processing while recording are off. The next
            check made on this computer, partial or complete, replaces the old one; what it allows depends on
            what it measured, including the recorder safety test.
            <div style={{ marginTop: 8 }}>
              <Button size="sm" onClick={() => void capability.run(false)}>
                Re-run Quick Check
              </Button>
            </div>
          </Notice>
        )}
        {view && view.saved_checks.length > 1 && (
          <Notice tone="info" title="A partial check is kept apart">
            {view.saved_checks.find((c) => c.file === "partial")?.in_use
              ? "The complete check belongs to an earlier state of this computer, so the partial check made on this computer is in use, with safe defaults."
              : "While the complete check still fits this computer, a partial check never replaces it."}
          </Notice>
        )}
        {cap && cap.injectors.length > 0 && (
          <Notice tone="warning" title="A screen-overlay tool is running">
            {cap.injectors.join(" · ")}. These tools can crash the display and disturb timing. Close them
            before recording or running a full check.
          </Notice>
        )}
        {view?.error && (
          <Notice tone="warning" title="The saved profile could not be used">
            {view.error}. Run the check again.
          </Notice>
        )}
        <Section title="Hardware" first>
          {cap ? (
            <Table
              headers={["Part", "Found", "Result"]}
              rows={hardwareRows(cap).map((r) => [
                { content: r.part, strong: true },
                { content: r.found, mono: r.mono },
                { content: <Badge tone={r.result.tone}>{r.result.label}</Badge> },
              ])}
            />
          ) : (
            <Table
              headers={["Part", "Found", "Result"]}
              rows={["Processor", "Memory", "Graphics", "Storage and power", "Audio input"].map((part) => [
                { content: part, strong: true },
                { content: "Not checked yet" },
                { content: <Badge>Pending</Badge> },
              ])}
            />
          )}
        </Section>
        <Section title="Speed test">
          <Table
            headers={["Model", "Role", "Devices", "Speed", "Memory", "Result"]}
            rows={
              cap && cap.measurements.length > 0
                ? speedRows(cap).map((r) => [
                    { content: r.model, mono: true },
                    { content: r.role },
                    { content: r.devices, mono: true },
                    { content: r.speed, mono: true },
                    { content: r.memory, mono: true },
                    { content: r.result ? <Badge tone={r.result.tone}>{r.result.label}</Badge> : "" },
                  ])
                : [
                    [
                      { content: "No models measured" },
                      { content: "" },
                      { content: "" },
                      { content: "" },
                      { content: "" },
                      { content: <Badge>Pending</Badge> },
                    ],
                  ]
            }
          />
          {decision && decision.bars && (
            <Note style={{ marginTop: 6, fontSize: 12, color: "var(--text-muted)" }}>
              Speed is processing time per second of audio (lower is faster). Bars: recording passes up to{" "}
              {decision.bars.rolling_rtf.toFixed(2)}×, near-live up to{" "}
              {decision.bars.near_live_rtf.toFixed(2)}×, live captions up to{" "}
              {decision.bars.live_standin_rtf.toFixed(2)}×
              {cap?.emulated ? " (emulated bars: 25% headroom)" : ""}.
            </Note>
          )}
        </Section>
        <EnginesSection registry={registry} />
        <ModelsSection registry={registry} />
      </MainWithSide>
      <Dialog
        open={pending !== null}
        onClose={() => setPending(null)}
        eyebrow="Override"
        title={pending ? OVERRIDE_TEXT[pending.kind].label : ""}
        footer={
          <>
            <Button variant="secondary" onClick={() => setPending(null)}>
              Keep the Current Setting
            </Button>
            <Button
              onClick={() => {
                if (pending) void capability.setOverride(pending.kind, pending.enabled, true);
                setPending(null);
              }}
            >
              {pending?.enabled ? "Turn On" : "Turn Off"}
            </Button>
          </>
        }
      >
        <Note>{pending ? OVERRIDE_TEXT[pending.kind].risk : ""}</Note>
        <Note style={{ marginTop: 8, fontSize: 12, color: "var(--text-muted)" }}>
          The change is saved in the profile with the time, and shown on this screen.
        </Note>
      </Dialog>
    </>
  );
}

/** Items for the status bar when the System screen is open: when the profile was saved and its id. */
export function systemStatus(capability: CapabilityState): { items: string[]; trailing: string } {
  const cap = capability.view?.capability;
  return {
    items: cap
      ? [`capability.json saved ${savedTime(cap.created_at)}`, "Re-runs when hardware or models change"]
      : ["capability.json not saved yet"],
    trailing: capability.view?.fingerprint_id ? `Fingerprint ${capability.view.fingerprint_id}` : "",
  };
}
