import type { CSSProperties, ReactNode } from "react";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/navigation/Tabs.jsx).
export type TabItem = string | { value: string; label: ReactNode; meta?: ReactNode };

export interface TabsProps {
  items?: TabItem[];
  value?: string;
  onChange?: (value: string) => void;
  size?: "md" | "lg";
  style?: CSSProperties;
}

export function Tabs({ items = [], value, onChange, size = "md", style }: TabsProps) {
  return (
    <div
      role="tablist"
      style={{
        display: "flex",
        gap: 28,
        borderBottom: px(1) + " solid var(--ink-200)",
        ...style,
      }}
    >
      {items.map((it) => {
        const v = typeof it === "string" ? it : it.value;
        const l = typeof it === "string" ? it : it.label;
        const meta = typeof it === "string" ? null : it.meta;
        const on = v === value;
        return (
          <button
            key={v}
            role="tab"
            aria-selected={on}
            onClick={() => onChange && onChange(v)}
            style={{
              background: "none",
              border: "none",
              padding: size === "lg" ? px(12, 0, 14) : px(10, 0, 12),
              marginBottom: -1,
              cursor: "pointer",
              display: "flex",
              flexDirection: "column",
              alignItems: "flex-start",
              gap: 2,
              borderBottom: px(2) + " solid " + (on ? "var(--orange-600)" : "transparent"),
              color: on ? "var(--ink-900)" : "var(--ink-600)",
              transition: "color var(--dur-fast), border-color var(--dur-base) var(--ease-standard)",
            }}
          >
            <span
              style={{
                fontFamily: "var(--font-ui)",
                fontSize: size === "lg" ? 16 : 14,
                fontWeight: on ? 600 : 500,
              }}
            >
              {l}
            </span>
            {meta && (
              <span
                style={{
                  fontFamily: "var(--font-ui)",
                  fontSize: 12,
                  fontWeight: 600,
                  letterSpacing: "0.08em",
                  textTransform: "uppercase",
                  color: "var(--ink-500)",
                }}
              >
                {meta}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
