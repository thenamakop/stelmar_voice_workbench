import { useState } from "react";
import type { ReactNode } from "react";
import {
  Badge,
  Button,
  Icon,
  IconButton,
  ProgressBar,
  Select,
  Switch,
  Tabs,
  Tag,
} from "../ds";
import type { BadgeTone } from "../ds";
import { px } from "../ds/units";
import type { MirrorStatus, RecorderStatus, Snapshot } from "../ipc";
import { clock, istDateTime, languages, meterPercent, minutes, partFile } from "../recording/format";
import type { Level, Recorder } from "../recording/useRecorder";
import { KeyValue, MainWithSide, Note, PageHead, SideCard } from "../shell/layout";
import type { CapabilityState } from "../system/useCapability";
import { liveUnavailableReason } from "./liveReason";
import { StartSessionDialog } from "./StartSessionDialog";
import { NoEngineBanner, useRegistry } from "./RegistrySections";

// Record screen per design/mockups/record-only.html. Phase 1: input, meter, recording state,
// parts strip (Recording and Waiting until Phase 4 adds transcription) and backup. While no
// session is open, the rows are sample text that shows the English, Hindi, mixed and Urdu fonts.

type Lang = "en" | "hi" | "ur" | "mixed";

const SAMPLE_ROWS: { time: string; badge: string; lang: Lang; text: string }[] = [
  {
    time: "00:00:00",
    badge: "EN",
    lang: "en",
    text: "Our study compares rooftop solar output across three campus buildings over eighteen months.",
  },
  {
    time: "00:00:00",
    badge: "HI",
    lang: "hi",
    text: "इस अध्ययन में हमने तीन इमारतों की छतों पर लगे सोलर पैनलों के आँकड़े देखे।",
  },
  {
    time: "00:00:00",
    badge: "HI·EN",
    lang: "mixed",
    text: "तो हमारा main finding यह है कि battery storage से peak load काफ़ी कम हुआ।",
  },
  {
    time: "00:00:00",
    badge: "UR",
    lang: "ur",
    text: "اس مطالعے میں ہم نے تین عمارتوں کی چھتوں پر لگے سولر پینلوں کا ڈیٹا دیکھا۔",
  },
];

function rowText(lang: Lang, text: string): ReactNode {
  const base = { fontSize: 17, lineHeight: px(28), color: "var(--text-body)" } as const;
  switch (lang) {
    case "hi":
    case "mixed":
      return (
        <div
          lang="hi"
          dir={lang === "mixed" ? "auto" : undefined}
          style={{
            ...base,
            fontFamily: "var(--font-devanagari)",
            fontSize: 18,
            lineHeight: "var(--lh-devanagari)",
          }}
        >
          {text}
        </div>
      );
    case "ur":
      return (
        <div
          lang="ur"
          dir="rtl"
          style={{
            ...base,
            fontFamily: "var(--font-urdu)",
            fontSize: 19,
            lineHeight: "var(--lh-urdu)",
            textAlign: "right",
          }}
        >
          {text}
        </div>
      );
    default:
      return <div style={base}>{text}</div>;
  }
}

function Row({ time, badge, children }: { time: string; badge: ReactNode; children: ReactNode }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: px(80) + " " + px(40) + " 1fr " + px(28),
        gap: 14,
        alignItems: "start",
        padding: "var(--space-4) 0",
        borderBottom: "var(--border-w) solid var(--paper-200)",
      }}
    >
      <span
        style={{ fontFamily: "var(--font-mono)", fontSize: 13, lineHeight: px(28), color: "var(--ink-500)" }}
      >
        {time}
      </span>
      <span style={{ paddingTop: 3 }}>{badge}</span>
      {children}
      <span />
    </div>
  );
}

/** Peak-hold meter (.meter in mockup.css): RMS bar, held-peak tick, −60 to 0 dBFS. */
function Meter({ level }: { level: Level | null }) {
  const rms = level ? meterPercent(level.rms) : 0;
  const hold = level ? meterPercent(level.hold) : 0;
  const hot = !!level && (level.clipped || level.hold > -1);
  const loud = !!level && level.hold > -6;
  const fill = hot ? "var(--status-danger-fg)" : loud ? "var(--amber-500)" : "var(--green-500)";
  return (
    <>
      <div
        role="meter"
        aria-label="Input level"
        aria-valuemin={-60}
        aria-valuemax={0}
        aria-valuenow={level ? Math.round(level.rms) : -60}
        style={{
          height: 10,
          borderRadius: 2,
          background: "var(--paper-200)",
          position: "relative",
          overflow: "hidden",
          margin: px(6, 0),
        }}
      >
        <i style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: `${rms}%`, background: fill }} />
        {level && (
          <s
            style={{
              position: "absolute",
              top: 0,
              bottom: 0,
              left: `calc(${hold}% - ${px(2)})`,
              width: 2,
              background: hot ? "var(--red-700)" : "var(--ink-900)",
            }}
          />
        )}
      </div>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          fontFamily: "var(--font-mono)",
          fontSize: 11,
          color: "var(--ink-500)",
        }}
      >
        <span>−60</span>
        <span>−24</span>
        <span>−12 peak</span>
        <span>0 dBFS</span>
      </div>
    </>
  );
}

type PartTone = "rec" | "wait" | "paused" | "lost";

/** One tile of the parts strip (.part in mockup.css). */
function PartTile({ range, label, tone }: { range: string; label: string; tone: PartTone }) {
  const styles: Record<PartTone, { background: string; border: string; color: string }> = {
    rec: { background: "var(--status-danger-bg)", border: "transparent", color: "var(--status-danger-fg)" },
    wait: { background: "var(--paper-100)", border: "var(--border-hairline)", color: "var(--ink-600)" },
    paused: {
      background: "var(--status-warning-bg)",
      border: "transparent",
      color: "var(--status-warning-fg)",
    },
    lost: { background: "var(--status-danger-bg)", border: "transparent", color: "var(--status-danger-fg)" },
  };
  const s = styles[tone];
  return (
    <div
      style={{
        borderRadius: "var(--radius-xs)",
        padding: px(7, 10),
        background: s.background,
        border: `var(--border-w) solid ${s.border}`,
        fontSize: 12,
        lineHeight: px(16),
        color: s.color,
      }}
    >
      <b
        style={{
          display: "block",
          fontFamily: "var(--font-mono)",
          fontWeight: "var(--fw-medium)",
          fontSize: 12,
          color: "var(--ink-900)",
        }}
      >
        {range}
      </b>
      {label}
    </div>
  );
}

function PartsStrip({ snapshot }: { snapshot: Snapshot }) {
  const rec = snapshot.recorder;
  const tiles: { key: string; range: string; label: string; tone: PartTone }[] = snapshot.parts.map((p) => ({
    key: `${p.part}-${p.start_ms}`,
    range: `${minutes(p.start_ms)}–${minutes(p.start_ms + p.duration_ms)}`,
    label: "Waiting",
    tone: "wait",
  }));
  if (rec && rec.state !== "stopped") {
    const start = rec.audio_ms - rec.part_audio_ms;
    tiles.push({
      key: "current",
      range: `${minutes(start)}–`,
      label:
        rec.state === "recording" && rec.part !== null
          ? "Recording"
          : rec.state === "paused"
            ? "Paused"
            : "Waiting for input",
      tone:
        rec.state === "recording" && rec.part !== null
          ? "rec"
          : rec.state === "input_lost"
            ? "lost"
            : rec.state === "paused"
              ? "paused"
              : "wait",
    });
  }
  const shown = tiles.slice(-5);
  return (
    <div
      aria-label="Parts"
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(5, minmax(0, 1fr))",
        gap: 6,
        margin: px(10, 0, 6),
      }}
    >
      {shown.map((t) => (
        <PartTile key={t.key} range={t.range} label={t.label} tone={t.tone} />
      ))}
    </div>
  );
}

function mirrorBadge(
  m: MirrorStatus | null,
  hasBackup: boolean,
  recording: boolean,
  stalled: boolean,
): ReactNode {
  if (recording && hasBackup && stalled) return <Badge tone="warning">Not responding</Badge>;
  if (!m) return <Badge>{!hasBackup ? "Not set" : recording ? "Starting" : "Ready"}</Badge>;
  const map: Record<MirrorStatus["state"], [BadgeTone, string]> = {
    not_set: ["neutral", "Not set"],
    in_sync: ["success", "In sync"],
    behind: [m.warn ? "warning" : "neutral", "Behind"],
    missing: ["danger", "Drive missing"],
    error: ["danger", "Error"],
  };
  const [tone, text] = map[m.state];
  return <Badge tone={tone}>{text}</Badge>;
}

/** "1,200 samples, 1 driver glitch": only the non-zero parts, each pluralised. */
function lostAudioText(r: RecorderStatus): string {
  const lost = r.overrun_samples + r.dropped_samples;
  const bits: string[] = [];
  if (lost > 0) bits.push(`${lost.toLocaleString("en-IN")} sample${lost === 1 ? "" : "s"}`);
  if (r.xruns > 0) bits.push(`${r.xruns} driver glitch${r.xruns === 1 ? "" : "es"}`);
  return bits.join(", ");
}

function stateBadge(r: RecorderStatus | null): ReactNode {
  if (!r)
    return (
      <Badge tone="danger" dot>
        Rec
      </Badge>
    );
  switch (r.state) {
    case "recording":
      return (
        <Badge tone="danger" dot>
          Rec
        </Badge>
      );
    case "paused":
      return (
        <Badge tone="warning" dot>
          Paused
        </Badge>
      );
    case "input_lost":
      return (
        <Badge tone="danger" dot>
          Input lost
        </Badge>
      );
    default:
      return <Badge>Stopped</Badge>;
  }
}

export interface RecordScreenProps {
  hasProfile: boolean;
  recorder: Recorder;
  capability: CapabilityState;
}

export function RecordScreen({ hasProfile, recorder, capability }: RecordScreenProps) {
  const [tab, setTab] = useState("transcript");
  const [startOpen, setStartOpen] = useState(false);
  const { snapshot, level, inputs, settings, busy } = recorder;
  const registry = useRegistry();
  const session = snapshot?.session ?? null;
  const rec = snapshot?.recorder ?? null;
  const mirror = snapshot?.mirror ?? null;
  const recording = !!session;

  // The operator's choice, else the last device used, else the first input (the default).
  const [chosenDevice, setDeviceId] = useState("");
  const deviceId = inputs.some((d) => d.id === chosenDevice)
    ? chosenDevice
    : ((inputs.find((d) => d.id === settings?.last_device_id) ?? inputs[0])?.id ?? "");

  const state = rec?.state ?? (recording ? "recording" : "stopped");
  const overline = !recording
    ? "Not recording"
    : `${state === "paused" ? "Paused" : state === "input_lost" ? "Input lost" : "Recording"}${session.hall ? ` · ${session.hall}` : ""}`;
  const partNo = rec?.part ?? null;
  const partFraction = rec && rec.part !== null ? rec.part_audio_ms / (rec.part_seconds * 1000) : 0;
  const partLeftMin = rec ? Math.max(0, Math.ceil((rec.part_seconds * 1000 - rec.part_audio_ms) / 60000)) : 0;
  const input = rec?.input ?? session?.input ?? null;

  return (
    <>
      <PageHead
        overline={overline}
        title={session ? session.title : "No Session Open"}
        meta={
          session ? (
            <>
              <span style={{ fontFamily: "var(--font-mono)" }}>
                {session.scheduled ?? istDateTime(session.opened_at)}
              </span>
              <Badge>Session</Badge>
              <Badge tone="info">Record + post-process</Badge>
              <Badge>{languages(session.expected_languages)}</Badge>
            </>
          ) : (
            <>
              <span>Pick a session on the Sessions screen</span>
              <Badge tone="info">Record + post-process</Badge>
              <Badge>EN · HI</Badge>
            </>
          )
        }
        controls={
          <>
            {recording && stateBadge(rec)}
            <span
              aria-label="Recorded time"
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 24,
                fontWeight: "var(--fw-medium)",
                color: "var(--ink-900)",
                minWidth: 122,
                textAlign: "right",
              }}
            >
              {clock(rec?.audio_ms ?? 0)}
            </span>
            <IconButton icon="bookmark" label="Bookmark this moment" disabled />
            {recording && state === "paused" ? (
              <Button
                variant="secondary"
                iconLeft="play"
                disabled={busy}
                onClick={() => void recorder.resume()}
              >
                Resume
              </Button>
            ) : (
              <Button
                variant="secondary"
                iconLeft="pause"
                disabled={!recording || busy || state !== "recording"}
                onClick={() => void recorder.pause()}
              >
                Pause
              </Button>
            )}
            {recording ? (
              <Button iconLeft="square" disabled={busy} onClick={() => void recorder.stop()}>
                Stop Session
              </Button>
            ) : (
              <Button
                iconLeft="circle"
                disabled={busy || inputs.length === 0}
                onClick={() => setStartOpen(true)}
              >
                Start Session
              </Button>
            )}
          </>
        }
      />
      <MainWithSide
        side={
          <>
            <SideCard title="Input">
              {recording || inputs.length === 0 ? (
                <KeyValue rows={[["Device", input ? input.device_name : "No input found"]]} />
              ) : (
                <Select
                  aria-label="Audio input"
                  value={deviceId}
                  onChange={(e) => setDeviceId(e.target.value)}
                  options={inputs.map((d) => ({ value: d.id, label: d.name }))}
                />
              )}
              <Meter level={recording && state === "recording" ? level : null} />
              {rec && rec.overrun_samples + rec.dropped_samples + rec.xruns > 0 && (
                <div
                  style={{
                    display: "flex",
                    gap: 8,
                    alignItems: "center",
                    marginTop: 8,
                    fontSize: 12,
                    color: "var(--status-danger-fg)",
                  }}
                >
                  <Icon name="alert-triangle" size={14} />
                  <span>Audio lost: {lostAudioText(rec)}</span>
                </div>
              )}
            </SideCard>
            <SideCard title="Language">
              <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
                <Tag selected>English</Tag>
                <Tag selected>Hindi</Tag>
                <Tag>Urdu</Tag>
              </div>
              <div style={{ fontSize: 12, color: "var(--text-muted)", marginTop: 8 }}>
                Per sentence · Alt+1/2/3 lock · Alt+4 detect
              </div>
            </SideCard>
            <SideCard title="Transcription">
              <Switch label="Live captions" disabled />
              <div
                style={{ fontSize: 12, lineHeight: px(18), color: "var(--text-muted)", margin: px(6, 0, 10) }}
              >
                {liveUnavailableReason(hasProfile)}
              </div>
              <KeyValue
                rows={[
                  ["Devices", "Not measured"],
                  ["Speed", "Not measured"],
                  ["Threads", "Not measured"],
                ]}
              />
            </SideCard>
            <SideCard title="Backup">
              <KeyValue
                rows={[
                  [
                    "Audio",
                    recording && partNo !== null ? partFile(partNo) : recording ? "Waiting" : "No recording",
                    { mono: recording && partNo !== null },
                  ],
                  [
                    "Mirror",
                    mirrorBadge(
                      recording ? mirror : null,
                      !!(session?.backup_root ?? settings?.backup_root),
                      recording,
                      recorder.mirrorStalled,
                    ),
                  ],
                  ...(recording && mirror && mirror.state !== "not_set" && mirror.lag_ms >= 2000
                    ? ([["Behind", `${Math.round(mirror.lag_ms / 1000)} s`, { mono: true }]] as [
                        ReactNode,
                        ReactNode,
                        { mono?: boolean },
                      ][])
                    : []),
                ]}
              />
            </SideCard>
          </>
        }
      >
        <Tabs
          value={tab}
          onChange={setTab}
          items={[
            { value: "transcript", label: "Transcript" },
            { value: "bookmarks", label: "Bookmarks" },
            { value: "speakers", label: "Speakers" },
          ]}
        />
        <NoEngineBanner registry={registry} />
        {recording && state === "input_lost" && (
          <div
            role="alert"
            style={{
              display: "flex",
              gap: 10,
              alignItems: "center",
              marginTop: 12,
              padding: px(10, 14),
              borderRadius: "var(--radius-sm)",
              background: "var(--status-danger-bg)",
              color: "var(--status-danger-fg)",
              fontSize: 13,
              lineHeight: "var(--lh-small)",
            }}
          >
            <Icon name="alert-triangle" size={16} />
            <span>
              <b style={{ fontWeight: "var(--fw-semibold)" }}>Input lost.</b> Retrying every 2 seconds.
              Recording continues in a new part when {input ? input.device_name : "the input"} returns.
            </span>
          </div>
        )}
        {recording && rec?.disk_warning && (
          <div style={{ display: "flex", gap: 10, alignItems: "center", padding: px(12, 0, 2) }}>
            <Icon name="alert-triangle" size={16} color="var(--status-warning-fg)" />
            <Note style={{ color: "var(--status-warning-fg)" }}>
              Less than 20 GB free on the recording drive.
            </Note>
          </div>
        )}
        <div style={{ display: "flex", gap: 10, alignItems: "center", padding: px(12, 0, 2) }}>
          <Icon name={recording ? "clock" : "info"} size={16} color="var(--text-secondary)" />
          <Note>
            {recording
              ? session.backup_root
                ? "Recording to this machine in 10-minute parts, copied to the backup folder as they are written. Transcription of closed parts comes in a later update."
                : "Recording to this machine in 10-minute parts. No backup folder is set. Transcription of closed parts comes in a later update."
              : "Transcription is not built yet. The lines below are sample text that checks the English, Hindi, mixed and Urdu fonts."}
          </Note>
        </div>
        {recording && snapshot && <PartsStrip snapshot={snapshot} />}
        <div style={{ paddingTop: 6 }}>
          {recording ? (
            <Row
              time={clock(rec ? rec.audio_ms - rec.part_audio_ms : 0)}
              badge={<Icon name="loader" size={16} color="var(--ink-500)" />}
            >
              <div style={{ fontSize: 14, lineHeight: px(22), color: "var(--text-secondary)" }}>
                {partNo !== null
                  ? `Part ${partNo} closes in about ${partLeftMin} min`
                  : state === "paused"
                    ? "Paused. Resume starts a new part."
                    : "Waiting for audio"}
                <ProgressBar
                  value={partFraction}
                  label={partNo !== null ? `Part ${partNo} recorded` : "Current part recorded"}
                  style={{ marginTop: 8, maxWidth: 420 }}
                />
              </div>
            </Row>
          ) : (
            <>
              {SAMPLE_ROWS.map((r) => (
                <Row key={r.badge} time={r.time} badge={<Badge>{r.badge}</Badge>}>
                  {rowText(r.lang, r.text)}
                </Row>
              ))}
              <Row time="00:00:00" badge={<Icon name="loader" size={16} color="var(--ink-500)" />}>
                <div style={{ fontSize: 14, lineHeight: px(22), color: "var(--text-secondary)" }}>
                  Waiting for the first 10-minute part
                  <ProgressBar
                    value={0}
                    label="Part 1 transcription"
                    style={{ marginTop: 8, maxWidth: 420 }}
                  />
                </div>
              </Row>
            </>
          )}
        </div>
      </MainWithSide>
      <StartSessionDialog
        open={startOpen}
        onClose={() => setStartOpen(false)}
        inputs={inputs}
        deviceId={deviceId}
        onDevice={setDeviceId}
        busy={busy}
        checkRunning={capability.running}
        profileKey={capability.view?.capability?.created_at ?? "none"}
        onStart={(choice) => {
          void recorder
            .start({
              title: choice.title,
              hall: choice.hall || null,
              device_id: deviceId || null,
              expected_languages: choice.expected_languages,
              mode: choice.mode,
            })
            .then((ok) => {
              if (ok) setStartOpen(false);
            });
        }}
      />
    </>
  );
}
