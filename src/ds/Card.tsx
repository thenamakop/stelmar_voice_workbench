import { useState } from "react";
import type { CSSProperties, HTMLAttributes, MouseEventHandler, ReactNode } from "react";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/layout/Card.jsx).
export type CardVariant = "outline" | "muted" | "ruled" | "inverse";

export interface CardProps extends Omit<HTMLAttributes<HTMLDivElement>, "onClick"> {
  variant?: CardVariant;
  padding?: number | string;
  interactive?: boolean;
  onClick?: MouseEventHandler<HTMLDivElement>;
  children?: ReactNode;
  style?: CSSProperties;
}

const VARIANTS: Record<CardVariant, CSSProperties> = {
  outline: { background: "var(--surface-card)", border: px(1) + " solid var(--border-hairline)" },
  muted: { background: "var(--surface-muted)", border: px(1) + " solid transparent" },
  ruled: {
    background: "transparent",
    border: "none",
    borderTop: px(2) + " solid var(--ink-900)",
    borderRadius: 0,
  },
  inverse: {
    background: "var(--surface-inverse)",
    border: px(1) + " solid transparent",
    color: "var(--text-inverse)",
  },
};

export function Card({
  variant = "outline",
  padding = 24,
  interactive,
  onClick,
  children,
  style,
  ...rest
}: CardProps) {
  const [hover, setHover] = useState(false);
  return (
    <div
      {...rest}
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        borderRadius: "var(--radius-md)",
        padding,
        boxSizing: "border-box",
        ...VARIANTS[variant],
        ...(interactive && hover ? { borderColor: "var(--ink-900)", boxShadow: "var(--shadow-1)" } : {}),
        cursor: interactive ? "pointer" : undefined,
        transition: "border-color var(--dur-fast) var(--ease-standard), box-shadow var(--dur-fast)",
        ...style,
      }}
    >
      {children}
    </div>
  );
}
