import { useId, useState } from "react";
import type { CSSProperties, ReactNode, SelectHTMLAttributes } from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/forms/Select.jsx).
const labelStyle: CSSProperties = {
  fontFamily: "var(--font-ui)",
  fontSize: 13,
  fontWeight: 500,
  color: "var(--ink-900)",
};

const hintStyle = (err: boolean): CSSProperties => ({
  fontFamily: "var(--font-ui)",
  fontSize: 12,
  lineHeight: px(16),
  color: err ? "var(--red-600)" : "var(--ink-500)",
});

export type SelectOption = string | { value: string; label: ReactNode };

export interface SelectProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "style"> {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  options?: SelectOption[];
  placeholder?: string;
  required?: boolean;
  disabled?: boolean;
  id?: string;
  style?: CSSProperties;
}

export function Select({
  label,
  hint,
  error,
  options = [],
  placeholder,
  required,
  disabled,
  id,
  style,
  ...rest
}: SelectProps) {
  const [focus, setFocus] = useState(false);
  const generatedId = useId();
  const fid = id || generatedId;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, ...style }}>
      {label && (
        <label htmlFor={fid} style={labelStyle}>
          {label}
          {required && <span style={{ color: "var(--orange-600)" }}> *</span>}
        </label>
      )}
      <div style={{ position: "relative" }}>
        <select
          id={fid}
          disabled={disabled}
          required={required}
          {...rest}
          onFocus={() => setFocus(true)}
          onBlur={() => setFocus(false)}
          style={{
            width: "100%",
            height: 40,
            appearance: "none",
            WebkitAppearance: "none",
            padding: px(0, 36, 0, 12),
            fontFamily: "var(--font-ui)",
            fontSize: 14,
            color: "var(--ink-900)",
            background: disabled ? "var(--paper-100)" : "var(--paper-0)",
            border:
              px(1) + " solid " + (error ? "var(--red-600)" : focus ? "var(--blue-600)" : "var(--ink-300)"),
            borderRadius: "var(--radius-sm)",
            outline: "none",
            boxShadow: focus ? "0 0 0 " + px(3) + " var(--blue-100)" : "none",
            cursor: "pointer",
          }}
        >
          {placeholder && <option value="">{placeholder}</option>}
          {options.map((o) => {
            const v = typeof o === "string" ? o : o.value;
            const l = typeof o === "string" ? o : o.label;
            return (
              <option key={v} value={v}>
                {l}
              </option>
            );
          })}
        </select>
        <Icon
          name="chevron-down"
          size={16}
          color="var(--ink-600)"
          style={{ position: "absolute", right: 12, top: 12, pointerEvents: "none" }}
        />
      </div>
      {(error || hint) && <div style={hintStyle(!!error)}>{error || hint}</div>}
    </div>
  );
}
