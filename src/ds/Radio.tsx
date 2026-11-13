import type { CSSProperties, ReactNode } from "react";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/forms/Radio.jsx).
export interface RadioProps {
  checked?: boolean;
  onChange?: (value: string) => void;
  label?: ReactNode;
  description?: ReactNode;
  name?: string;
  value: string;
  disabled?: boolean;
  style?: CSSProperties;
}

export function Radio({ checked, onChange, label, description, name, value, disabled, style }: RadioProps) {
  return (
    <label
      style={{
        display: "flex",
        gap: 10,
        alignItems: "flex-start",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.5 : 1,
        ...style,
      }}
    >
      <input
        type="radio"
        name={name}
        value={value}
        checked={!!checked}
        disabled={disabled}
        onChange={() => onChange && onChange(value)}
        style={{ position: "absolute", opacity: 0, width: 0, height: 0 }}
      />
      <span
        style={{
          width: 18,
          height: 18,
          marginTop: 2,
          flex: "none",
          boxSizing: "border-box",
          borderRadius: 999,
          border: px(1) + " solid " + (checked ? "var(--ink-900)" : "var(--ink-400)"),
          background: "var(--paper-0)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        {checked && <span style={{ width: 8, height: 8, borderRadius: 999, background: "var(--ink-900)" }} />}
      </span>
      <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        <span
          style={{
            fontFamily: "var(--font-ui)",
            fontSize: 14,
            lineHeight: px(22),
            color: "var(--ink-900)",
          }}
        >
          {label}
        </span>
        {description && (
          <span
            style={{
              fontFamily: "var(--font-ui)",
              fontSize: 13,
              lineHeight: px(18),
              color: "var(--ink-600)",
            }}
          >
            {description}
          </span>
        )}
      </span>
    </label>
  );
}
