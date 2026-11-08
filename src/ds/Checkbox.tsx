import type { CSSProperties, ReactNode } from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/forms/Checkbox.jsx).
export interface CheckboxProps {
  checked?: boolean;
  onChange?: (checked: boolean) => void;
  label?: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
  style?: CSSProperties;
}

export function Checkbox({ checked, onChange, label, description, disabled, style }: CheckboxProps) {
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
        type="checkbox"
        checked={!!checked}
        disabled={disabled}
        onChange={(e) => onChange && onChange(e.target.checked)}
        style={{ position: "absolute", opacity: 0, width: 0, height: 0 }}
      />
      <span
        style={{
          width: 18,
          height: 18,
          marginTop: 2,
          flex: "none",
          boxSizing: "border-box",
          borderRadius: "var(--radius-xs)",
          border: px(1) + " solid " + (checked ? "var(--ink-900)" : "var(--ink-400)"),
          background: checked ? "var(--ink-900)" : "var(--paper-0)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          transition: "background var(--dur-fast)",
        }}
      >
        {checked && <Icon name="check" size={14} color="var(--paper-50)" />}
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
