import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/core/Badge.jsx).
const T = {
  neutral: ["var(--status-neutral-fg)", "var(--status-neutral-bg)"],
  success: ["var(--status-success-fg)", "var(--status-success-bg)"],
  warning: ["var(--status-warning-fg)", "var(--status-warning-bg)"],
  danger: ["var(--status-danger-fg)", "var(--status-danger-bg)"],
  info: ["var(--status-info-fg)", "var(--status-info-bg)"],
  award: ["var(--status-award-fg)", "var(--status-award-bg)"],
  accent: ["var(--paper-0)", "var(--orange-600)"],
  ink: ["var(--paper-50)", "var(--ink-900)"],
} as const;

export type BadgeTone = keyof typeof T;

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone;
  dot?: boolean;
  children?: ReactNode;
  style?: CSSProperties;
}

export function Badge({ tone = "neutral", dot, children, style, ...rest }: BadgeProps) {
  const [fg, bg] = T[tone] ?? T.neutral;
  return (
    <span
      {...rest}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: 22,
        padding: px(0, 8),
        borderRadius: "var(--radius-xs)",
        background: bg,
        color: fg,
        fontFamily: "var(--font-ui)",
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: "0.06em",
        textTransform: "uppercase",
        whiteSpace: "nowrap",
        ...style,
      }}
    >
      {dot && <span style={{ width: 6, height: 6, borderRadius: 999, background: "currentColor" }} />}
      {children}
    </span>
  );
}
