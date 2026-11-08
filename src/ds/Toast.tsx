import type { CSSProperties, ReactNode } from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/feedback/Toast.jsx).
const T = {
  success: ["check-circle", "var(--green-600)"],
  info: ["info", "var(--blue-500)"],
  warning: ["alert-triangle", "var(--amber-600)"],
  danger: ["alert-circle", "var(--red-600)"],
} as const;

export type ToastTone = keyof typeof T;

export interface ToastProps {
  tone?: ToastTone;
  title?: ReactNode;
  message?: ReactNode;
  action?: ReactNode;
  onClose?: () => void;
  style?: CSSProperties;
}

export function Toast({ tone = "success", title, message, action, onClose, style }: ToastProps) {
  const [ic, c] = T[tone] ?? T.info;
  return (
    <div
      role="status"
      style={{
        display: "flex",
        gap: 12,
        alignItems: "flex-start",
        width: 380,
        maxWidth: "100%",
        boxSizing: "border-box",
        padding: px(14, 14, 14, 16),
        background: "var(--ink-900)",
        color: "var(--paper-50)",
        borderRadius: "var(--radius-md)",
        boxShadow: "var(--shadow-2)",
        ...style,
      }}
    >
      <Icon name={ic} size={18} color={c} style={{ marginTop: 2, filter: "brightness(1.4)" }} />
      <div style={{ flex: 1, minWidth: 0 }}>
        {title && (
          <div
            style={{
              fontFamily: "var(--font-ui)",
              fontSize: 14,
              fontWeight: 600,
              lineHeight: px(22),
            }}
          >
            {title}
          </div>
        )}
        {message && (
          <div
            style={{
              fontFamily: "var(--font-ui)",
              fontSize: 13,
              lineHeight: px(19),
              color: "var(--ink-300)",
            }}
          >
            {message}
          </div>
        )}
        {action && <div style={{ marginTop: 8 }}>{action}</div>}
      </div>
      {onClose && (
        <button
          aria-label="Dismiss"
          onClick={onClose}
          style={{
            background: "none",
            border: "none",
            color: "var(--ink-300)",
            cursor: "pointer",
            padding: 2,
            display: "flex",
          }}
        >
          <Icon name="x" size={16} />
        </button>
      )}
    </div>
  );
}
