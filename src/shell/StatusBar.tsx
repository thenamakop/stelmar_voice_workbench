import type { ReactNode } from "react";

// Status bar per design/mockups/record-only.html (.status).
export interface StatusBarProps {
  items: ReactNode[];
  trailing?: ReactNode;
  /** True when `trailing` is a path or fingerprint (IBM Plex Mono); plain text stays in Inter. */
  trailingMono?: boolean;
}

export function StatusBar({ items, trailing, trailingMono }: StatusBarProps) {
  return (
    <footer
      role="contentinfo"
      style={{
        background: "var(--ink-950)",
        color: "var(--ink-300)",
        fontSize: 12,
        display: "flex",
        alignItems: "center",
        gap: 24,
        padding: "0 var(--space-8)",
        whiteSpace: "nowrap",
        overflow: "hidden",
      }}
    >
      <span style={{ display: "flex", alignItems: "center", gap: 8, color: "var(--paper-0)", flex: "none" }}>
        <i
          aria-hidden="true"
          style={{ width: 7, height: 7, borderRadius: 999, background: "var(--green-500)" }}
        />
        Offline · no network connections
      </span>
      {items.map((item, i) => (
        <span key={i} style={{ flex: "none" }}>
          {item}
        </span>
      ))}
      {trailing && (
        <span
          title={typeof trailing === "string" ? trailing : undefined}
          style={{
            marginLeft: "auto",
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            fontFamily: trailingMono ? "var(--font-mono)" : undefined,
          }}
        >
          {trailing}
        </span>
      )}
    </footer>
  );
}
