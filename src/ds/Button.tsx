import { useState } from "react";
import type { ButtonHTMLAttributes, CSSProperties, ReactNode } from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/core/Button.jsx).
const SIZES = {
  sm: { h: 32, px: 12, fs: 13, ic: 16 },
  md: { h: 40, px: 16, fs: 15, ic: 18 },
  lg: { h: 54, px: 30, fs: 19, ic: 20 },
} as const;

const V = {
  primary: {
    bg: "var(--action-primary)",
    hov: "var(--action-primary-hover)",
    prs: "var(--action-primary-press)",
    fg: "var(--paper-50)",
    bd: "transparent",
  },
  accent: {
    bg: "var(--action-accent)",
    hov: "var(--action-accent-hover)",
    prs: "var(--action-accent-press)",
    fg: "var(--paper-0)",
    bd: "transparent",
  },
  secondary: {
    bg: "var(--paper-0)",
    hov: "var(--paper-100)",
    prs: "var(--paper-200)",
    fg: "var(--ink-900)",
    bd: "var(--ink-900)",
  },
  ghost: {
    bg: "transparent",
    hov: "var(--ink-100)",
    prs: "var(--ink-200)",
    fg: "var(--ink-900)",
    bd: "transparent",
  },
  light: {
    bg: "var(--action-light)",
    hov: "var(--action-light-hover)",
    prs: "var(--ink-400)",
    fg: "var(--ink-900)",
    bd: "transparent",
  },
  glass: {
    bg: "var(--glass-bg)",
    hov: "var(--glass-bg-strong)",
    prs: "var(--glass-bg-strong)",
    fg: "var(--paper-0)",
    bd: "var(--glass-border)",
  },
  inverse: {
    bg: "var(--paper-50)",
    hov: "var(--paper-200)",
    prs: "var(--paper-300)",
    fg: "var(--ink-900)",
    bd: "transparent",
  },
} as const;

export type ButtonVariant = keyof typeof V;
export type ButtonSize = keyof typeof SIZES;

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  iconLeft?: string;
  iconRight?: string;
  fullWidth?: boolean;
  children?: ReactNode;
  style?: CSSProperties;
}

export function Button({
  variant = "primary",
  size = "md",
  iconLeft,
  iconRight,
  fullWidth,
  disabled,
  type = "button",
  children,
  style,
  ...rest
}: ButtonProps) {
  const [hover, setHover] = useState(false);
  const [press, setPress] = useState(false);
  const s = SIZES[size] ?? SIZES.md;
  const v = V[variant] ?? V.primary;
  const bg = disabled
    ? variant === "ghost"
      ? "transparent"
      : "var(--ink-100)"
    : press
      ? v.prs
      : hover
        ? v.hov
        : v.bg;
  return (
    <button
      type={type}
      disabled={disabled}
      {...rest}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => {
        setHover(false);
        setPress(false);
      }}
      onMouseDown={() => setPress(true)}
      onMouseUp={() => setPress(false)}
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        gap: 8,
        height: s.h,
        padding: px(0, s.px),
        width: fullWidth ? "100%" : undefined,
        fontFamily: "var(--font-ui)",
        fontSize: s.fs,
        fontWeight: 500,
        whiteSpace: "nowrap",
        background: bg,
        color: disabled ? "var(--text-disabled)" : v.fg,
        border: px(1) + " solid " + (disabled ? "var(--ink-200)" : v.bd),
        borderRadius: "var(--radius-sm)",
        cursor: disabled ? "not-allowed" : "pointer",
        transition: "background var(--dur-fast) var(--ease-standard)",
        outline: "none",
        ...style,
      }}
    >
      {iconLeft && <Icon name={iconLeft} size={s.ic} />}
      {children}
      {iconRight && <Icon name={iconRight} size={s.ic} />}
    </button>
  );
}
