import type { CSSProperties, ReactNode } from "react";

// Ported from design/reference/ds-bundle.js (components/forms/Switch.jsx).
export interface SwitchProps {
  checked?: boolean;
  onChange?: (checked: boolean) => void;
  label?: ReactNode;
  disabled?: boolean;
  style?: CSSProperties;
}

export function Switch({ checked, onChange, label, disabled, style }: SwitchProps) {
  return (
    <label
      style={{
        display: "inline-flex",
        gap: 10,
        alignItems: "center",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.5 : 1,
        ...style,
      }}
    >
      <input
        type="checkbox"
        role="switch"
        checked={!!checked}
        disabled={disabled}
        onChange={(e) => onChange && onChange(e.target.checked)}
        style={{ position: "absolute", opacity: 0, width: 0, height: 0 }}
      />
      <span
        style={{
          width: 34,
          height: 20,
          borderRadius: 999,
          background: checked ? "var(--ink-900)" : "var(--ink-300)",
          position: "relative",
          transition: "background var(--dur-base) var(--ease-standard)",
          flex: "none",
        }}
      >
        <span
          style={{
            position: "absolute",
            top: 2,
            left: checked ? 16 : 2,
            width: 16,
            height: 16,
            borderRadius: 999,
            background: "var(--paper-0)",
            boxShadow: "var(--shadow-1)",
            transition: "left var(--dur-base) var(--ease-standard)",
          }}
        />
      </span>
      {label && (
        <span style={{ fontFamily: "var(--font-ui)", fontSize: 14, color: "var(--ink-900)" }}>{label}</span>
      )}
    </label>
  );
}
