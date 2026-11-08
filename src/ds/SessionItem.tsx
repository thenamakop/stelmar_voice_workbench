import type { CSSProperties, ReactNode } from "react";
import { Badge } from "./Badge";
import { IconButton } from "./IconButton";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/layout/SessionItem.jsx).
export interface SessionItemProps {
  start?: ReactNode;
  end?: ReactNode;
  kind?: string | null;
  title?: ReactNode;
  speakers?: ReactNode;
  room?: ReactNode;
  saved?: boolean;
  onToggleSave?: () => void;
  highlight?: boolean;
  style?: CSSProperties;
}

export function SessionItem({
  start,
  end,
  kind,
  title,
  speakers,
  room,
  saved,
  onToggleSave,
  highlight,
  style,
}: SessionItemProps) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: px(112) + " minmax(0,1fr) auto",
        gap: 20,
        padding: px(16, 0),
        borderBottom: px(1) + " solid var(--ink-200)",
        alignItems: "start",
        background: highlight ? "var(--paper-100)" : undefined,
        ...style,
      }}
    >
      <div
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: 13,
          lineHeight: px(22),
          color: "var(--ink-900)",
        }}
      >
        {start}
        {end && (
          <span style={{ color: "var(--ink-500)" }}>
            <br />
            {end}
          </span>
        )}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0 }}>
        {kind && (
          <div>
            <Badge tone={kind === "Keynote" ? "ink" : kind === "Award" ? "award" : "neutral"}>{kind}</Badge>
          </div>
        )}
        <div
          style={{
            fontFamily: "var(--font-heading)",
            fontSize: 19,
            lineHeight: px(25),
            fontWeight: 700,
            color: "var(--ink-900)",
            textWrap: "pretty",
          }}
        >
          {title}
        </div>
        {(speakers || room) && (
          <div
            style={{
              fontFamily: "var(--font-ui)",
              fontSize: 13,
              lineHeight: px(20),
              color: "var(--ink-600)",
            }}
          >
            {speakers}
            {speakers && room ? " · " : ""}
            {room}
          </div>
        )}
      </div>
      {onToggleSave && (
        <IconButton
          icon={saved ? "bookmark-check" : "bookmark"}
          label={saved ? "Remove from my programme" : "Save to my programme"}
          size="sm"
          onClick={onToggleSave}
          style={{ color: saved ? "var(--orange-600)" : undefined }}
        />
      )}
    </div>
  );
}
