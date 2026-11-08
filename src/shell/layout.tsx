import type { CSSProperties, ReactNode } from "react";
import { Card } from "../ds";
import { px } from "../ds/units";

// Screen layout pieces from design/mockups/mockup.css: .over, .head, .main, .pane, .side, .kv, .tbl.
// They arrange design-system components; they are not new components.

export function Overline({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return (
    <div
      style={{
        fontSize: "var(--fs-overline)",
        lineHeight: "var(--lh-overline)",
        fontWeight: "var(--fw-semibold)",
        letterSpacing: "0.1em",
        textTransform: "uppercase",
        color: "var(--ink-600)",
        display: "flex",
        alignItems: "center",
        gap: 10,
        ...style,
      }}
    >
      <span
        aria-hidden="true"
        style={{ width: 8, height: 8, borderRadius: 2, background: "var(--orange-500)", flex: "none" }}
      />
      {children}
    </div>
  );
}

export function PageHead({
  overline,
  title,
  meta,
  controls,
}: {
  overline: ReactNode;
  title: ReactNode;
  meta?: ReactNode;
  controls?: ReactNode;
}) {
  return (
    <section
      style={{
        display: "flex",
        alignItems: "flex-end",
        gap: "var(--space-7)",
        padding: px(22) + " var(--space-8) var(--space-6)",
        borderBottom: "var(--border-w) solid var(--border-hairline)",
        background: "var(--bg-page)",
      }}
    >
      <div>
        <Overline>{overline}</Overline>
        <h1
          style={{
            margin: px(8, 0, 6),
            fontSize: 28,
            lineHeight: px(36),
            fontWeight: "var(--fw-bold)",
            color: "var(--text-primary)",
            letterSpacing: "var(--ls-tight)",
          }}
        >
          {title}
        </h1>
        {meta && (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              fontSize: 13,
              lineHeight: "var(--lh-body)",
              color: "var(--text-secondary)",
            }}
          >
            {meta}
          </div>
        )}
      </div>
      {controls && (
        <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 12 }}>{controls}</div>
      )}
    </section>
  );
}

export function MainWithSide({ children, side }: { children: ReactNode; side?: ReactNode }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: side ? "1fr var(--app-sidebar-w)" : "1fr",
        minHeight: 0,
      }}
    >
      <section
        style={{
          padding: "var(--space-5) var(--space-8) 0",
          minHeight: 0,
          overflow: "auto",
          display: "flex",
          flexDirection: "column",
        }}
      >
        {children}
      </section>
      {side && (
        <aside
          style={{
            background: "var(--surface-muted)",
            borderLeft: "var(--border-w) solid var(--border-hairline)",
            padding: "var(--space-5)",
            display: "flex",
            flexDirection: "column",
            gap: 12,
            overflow: "auto",
          }}
        >
          {side}
        </aside>
      )}
    </div>
  );
}

/**
 * Sidebar card (.side .card). With `stack`, the heading and children are spaced evenly by a
 * 12 gap (.card.stack in mockup.css) instead of the heading's own bottom margin.
 */
export function SideCard({
  title,
  children,
  stack,
}: {
  title: ReactNode;
  children?: ReactNode;
  stack?: boolean;
}) {
  return (
    <Card
      padding={px(14, 16)}
      style={stack ? { display: "flex", flexDirection: "column", gap: 12 } : undefined}
    >
      <Overline style={{ marginBottom: stack ? 0 : 10 }}>{title}</Overline>
      {children}
    </Card>
  );
}

export function KeyValue({ rows }: { rows: [ReactNode, ReactNode, { mono?: boolean }?][] }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "auto 1fr",
        gap: px(6, 12),
        fontSize: 13,
        lineHeight: "var(--lh-small)",
        color: "var(--text-secondary)",
      }}
    >
      {rows.map(([k, v, opts], i) => (
        <KeyValueRow key={i} k={k} v={v} mono={!!opts?.mono} />
      ))}
    </div>
  );
}

function KeyValueRow({ k, v, mono }: { k: ReactNode; v: ReactNode; mono: boolean }) {
  return (
    <>
      <span>{k}</span>
      <b
        style={{
          fontWeight: "var(--fw-medium)",
          color: "var(--ink-900)",
          textAlign: "right",
          fontFamily: mono ? "var(--font-mono)" : undefined,
          fontSize: mono ? 12 : undefined,
        }}
      >
        {v}
      </b>
    </>
  );
}

export function Section({
  title,
  children,
  first,
}: {
  title: ReactNode;
  children: ReactNode;
  first?: boolean;
}) {
  return (
    <div style={{ marginTop: first ? 4 : 16 }}>
      <Overline style={{ marginBottom: 6 }}>{title}</Overline>
      {children}
    </div>
  );
}

const th: CSSProperties = {
  textAlign: "left",
  fontSize: 11,
  fontWeight: "var(--fw-semibold)",
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  color: "var(--ink-600)",
  padding: "var(--space-3) var(--space-4)",
  borderBottom: "var(--rule-heavy-w) solid var(--ink-900)",
};

const td: CSSProperties = {
  padding: "var(--space-3) var(--space-4)",
  borderBottom: "var(--border-w) solid var(--ink-200)",
  color: "var(--ink-800)",
  verticalAlign: "middle",
  lineHeight: "var(--lh-small)",
};

export interface TableCell {
  content: ReactNode;
  mono?: boolean;
  strong?: boolean;
}

/** App-density table (.tbl in mockup.css). The first column has no left padding. */
export function Table({ headers, rows }: { headers: string[]; rows: TableCell[][] }) {
  return (
    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 14 }}>
      <thead>
        <tr>
          {headers.map((h, i) => (
            <th key={h} style={{ ...th, paddingLeft: i === 0 ? 0 : 12 }}>
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, r) => (
          <tr key={r}>
            {row.map((cell, i) => (
              <td
                key={i}
                style={{
                  ...td,
                  paddingLeft: i === 0 ? 0 : 12,
                  fontFamily: cell.mono ? "var(--font-mono)" : undefined,
                  fontSize: cell.mono ? 13 : undefined,
                }}
              >
                {cell.strong ? (
                  <b style={{ color: "var(--ink-900)", fontWeight: "var(--fw-semibold)" }}>{cell.content}</b>
                ) : (
                  cell.content
                )}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Muted explanatory line (.notice / .para in mockup.css). */
export function Note({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return (
    <p
      style={{
        fontSize: 13,
        lineHeight: "var(--lh-small)",
        color: "var(--text-secondary)",
        margin: 0,
        ...style,
      }}
    >
      {children}
    </p>
  );
}
