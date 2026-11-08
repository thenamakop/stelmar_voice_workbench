import { useState } from "react";
import logoUrl from "../../design/brand/logo-lockup-header.png";
import { Badge } from "../ds";
import { ROUTES, type RouteId } from "../routes";

// Charcoal top bar per design/mockups/record-only.html (.bar): logo lockup on --surface-header,
// product label, website-style nav with the orange underline on the active item.
export interface TopBarProps {
  active: RouteId | null;
  onNavigate: (route: RouteId) => void;
  machineLabel: string;
  /** True when the label is a hardware name (IBM Plex Mono); plain sentences stay in Inter. */
  machineMono?: boolean;
}

function NavItem({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={on ? "page" : undefined}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex",
        alignItems: "center",
        position: "relative",
        padding: 0,
        background: "none",
        border: "none",
        cursor: "pointer",
        fontFamily: "var(--font-ui)",
        fontSize: 15,
        fontWeight: "var(--fw-medium)",
        color: on || hover ? "var(--paper-0)" : "var(--ink-200)",
        transition: "color var(--dur-fast) var(--ease-standard)",
      }}
    >
      {label}
      <span
        aria-hidden="true"
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 0,
          height: 2,
          background: on ? "var(--orange-500)" : "transparent",
        }}
      />
    </button>
  );
}

export function TopBar({ active, onNavigate, machineLabel, machineMono }: TopBarProps) {
  return (
    <header
      style={{
        background: "var(--surface-header)",
        color: "var(--paper-0)",
        display: "flex",
        alignItems: "center",
        gap: 28,
        padding: "0 var(--space-8)",
      }}
    >
      <img src={logoUrl} alt="STELMAR · BML Munjal University" style={{ height: 40, display: "block" }} />
      <span
        style={{
          fontSize: 12,
          fontWeight: "var(--fw-semibold)",
          letterSpacing: "0.1em",
          textTransform: "uppercase",
          color: "var(--ink-300)",
          paddingLeft: 20,
          borderLeft: "var(--border-w) solid var(--ink-600)",
          height: 24,
          display: "flex",
          alignItems: "center",
        }}
      >
        Voice Workbench
      </span>
      <nav
        aria-label="Main"
        style={{ display: "flex", alignItems: "stretch", height: "100%", gap: 32, marginLeft: 12 }}
      >
        {ROUTES.map((r) => (
          <NavItem key={r.id} label={r.label} on={r.id === active} onClick={() => onNavigate(r.id)} />
        ))}
      </nav>
      <div
        style={{
          marginLeft: "auto",
          display: "flex",
          alignItems: "center",
          gap: 16,
          fontSize: 13,
          color: "var(--ink-300)",
        }}
      >
        <Badge tone="success" dot>
          Offline
        </Badge>
        <span style={{ fontFamily: machineMono ? "var(--font-mono)" : undefined }}>{machineLabel}</span>
      </div>
    </header>
  );
}
