import { useState } from "react";
import type { ReactNode } from "react";
import { Badge, Button, Icon, IconButton, ProgressBar, Switch, Tabs, Tag } from "../ds";
import { px } from "../ds/units";
import { KeyValue, MainWithSide, Note, PageHead, SideCard } from "../shell/layout";
import { liveUnavailableReason } from "./liveReason";

// Record screen placeholder per design/mockups/record-only.html. No audio yet (Phase 1); the rows
// are sample text that shows the English, Hindi, mixed Hindi–English and Urdu fonts.

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
      return (
        <div
          lang="hi"
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
    case "mixed":
      return (
        <div
          lang="hi"
          dir="auto"
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
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: 13,
          lineHeight: px(28),
          color: "var(--ink-500)",
        }}
      >
        {time}
      </span>
      <span style={{ paddingTop: 3 }}>{badge}</span>
      {children}
      <span />
    </div>
  );
}

export interface RecordScreenProps {
  hasProfile: boolean;
}

export function RecordScreen({ hasProfile }: RecordScreenProps) {
  const [tab, setTab] = useState("transcript");
  return (
    <>
      <PageHead
        overline="Not recording"
        title="No Session Open"
        meta={
          <>
            <span>Pick a session on the Sessions screen</span>
            <Badge tone="info">Record + post-process</Badge>
            <Badge>EN · HI</Badge>
          </>
        }
        controls={
          <>
            <span
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 24,
                fontWeight: "var(--fw-medium)",
                color: "var(--ink-900)",
                minWidth: 122,
                textAlign: "right",
              }}
            >
              00:00:00
            </span>
            <IconButton icon="bookmark" label="Bookmark this moment" disabled />
            <Button variant="secondary" iconLeft="pause" disabled>
              Pause
            </Button>
            <Button iconLeft="circle" disabled>
              Start Session
            </Button>
          </>
        }
      />
      <MainWithSide
        side={
          <>
            <SideCard title="Input">
              <KeyValue rows={[["Device", "Not selected"]]} />
              <div
                style={{
                  height: 10,
                  borderRadius: 2,
                  background: "var(--paper-200)",
                  margin: px(6, 0),
                }}
                aria-label="Input level"
              />
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
                  ["Audio", "No recording"],
                  ["Mirror", <Badge key="m">Not set</Badge>],
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
        <div style={{ display: "flex", gap: 10, alignItems: "center", padding: px(12, 0, 2) }}>
          <Icon name="info" size={16} color="var(--text-secondary)" />
          <Note>
            Recording and transcription are not built yet. The lines below are sample text that checks the
            English, Hindi, mixed and Urdu fonts.
          </Note>
        </div>
        <div style={{ paddingTop: 6 }}>
          {SAMPLE_ROWS.map((r) => (
            <Row key={r.badge} time={r.time} badge={<Badge>{r.badge}</Badge>}>
              {rowText(r.lang, r.text)}
            </Row>
          ))}
          <Row time="00:00:00" badge={<Icon name="loader" size={16} color="var(--ink-500)" />}>
            <div style={{ fontSize: 14, lineHeight: px(22), color: "var(--text-secondary)" }}>
              Waiting for the first 10-minute part
              <ProgressBar value={0} label="Part 1 transcription" style={{ marginTop: 8, maxWidth: 420 }} />
            </div>
          </Row>
        </div>
      </MainWithSide>
    </>
  );
}
