import { useState } from "react";
import type { ButtonHTMLAttributes, CSSProperties } from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/core/IconButton.jsx).
export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: string;
  label: string;
  variant?: "ghost" | "secondary" | "primary";
  size?: "sm" | "md" | "lg";
  style?: CSSProperties;
}

const DIAMETER = { sm: 32, md: 40, lg: 48 } as const;

export function IconButton({
  icon,
  label,
  variant = "ghost",
  size = "md",
  disabled,
  style,
  ...rest
}: IconButtonProps) {
  const [hover, setHover] = useState(false);
  const d = DIAMETER[size] ?? 40;
  const fill = variant === "primary";
  const outline = variant === "secondary";
  const bg = fill
    ? hover
      ? "var(--action-primary-hover)"
      : "var(--action-primary)"
    : hover
      ? "var(--ink-100)"
      : outline
        ? "var(--paper-0)"
        : "transparent";
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      {...rest}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: d,
        height: d,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 0,
        borderRadius: "var(--radius-sm)",
        background: disabled ? "transparent" : bg,
        color: disabled ? "var(--text-disabled)" : fill ? "var(--paper-50)" : "var(--ink-900)",
        border: px(1) + " solid " + (outline ? "var(--ink-300)" : "transparent"),
        cursor: disabled ? "not-allowed" : "pointer",
        transition: "background var(--dur-fast) var(--ease-standard)",
        ...style,
      }}
    >
      <Icon name={icon} size={size === "sm" ? 16 : 18} />
    </button>
  );
}
