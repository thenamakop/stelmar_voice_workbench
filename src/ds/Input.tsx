import { useId, useState } from "react";
import type {
  CSSProperties,
  FocusEvent,
  InputHTMLAttributes,
  ReactNode,
  TextareaHTMLAttributes,
} from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/forms/Input.jsx).
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

type FieldElement = HTMLInputElement & HTMLTextAreaElement;
type SharedAttributes = Omit<
  InputHTMLAttributes<FieldElement> & TextareaHTMLAttributes<FieldElement>,
  "style"
>;

export interface InputProps extends SharedAttributes {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  iconLeft?: string;
  multiline?: boolean;
  rows?: number;
  required?: boolean;
  disabled?: boolean;
  id?: string;
  style?: CSSProperties;
}

export function Input({
  label,
  hint,
  error,
  iconLeft,
  multiline,
  rows = 4,
  required,
  disabled,
  id,
  style,
  ...rest
}: InputProps) {
  const [focus, setFocus] = useState(false);
  const generatedId = useId();
  const fid = id || generatedId;
  const bd = error ? "var(--red-600)" : focus ? "var(--blue-600)" : "var(--ink-300)";
  const Field = multiline ? "textarea" : "input";
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, ...style }}>
      {label && (
        <label htmlFor={fid} style={labelStyle}>
          {label}
          {required && <span style={{ color: "var(--orange-600)" }}> *</span>}
        </label>
      )}
      <div style={{ position: "relative", display: "flex" }}>
        {iconLeft && (
          <Icon
            name={iconLeft}
            size={16}
            color="var(--ink-500)"
            style={{ position: "absolute", left: 12, top: 12 }}
          />
        )}
        <Field
          id={fid}
          rows={multiline ? rows : undefined}
          disabled={disabled}
          required={required}
          {...rest}
          onFocus={(e: FocusEvent<FieldElement>) => {
            setFocus(true);
            rest.onFocus?.(e);
          }}
          onBlur={(e: FocusEvent<FieldElement>) => {
            setFocus(false);
            rest.onBlur?.(e);
          }}
          style={{
            width: "100%",
            boxSizing: "border-box",
            height: multiline ? "auto" : 40,
            padding: multiline ? px(10, 12) : px(0, 12),
            paddingLeft: iconLeft ? 36 : 12,
            fontFamily: "var(--font-ui)",
            fontSize: 14,
            lineHeight: px(22),
            color: "var(--ink-900)",
            background: disabled ? "var(--paper-100)" : "var(--paper-0)",
            border: px(1) + " solid " + bd,
            borderRadius: "var(--radius-sm)",
            outline: "none",
            boxShadow: focus
              ? "0 0 0 " + px(3) + " " + (error ? "var(--red-100)" : "var(--blue-100)")
              : "none",
            resize: multiline ? "vertical" : undefined,
            transition: "border-color var(--dur-fast), box-shadow var(--dur-fast)",
          }}
        />
      </div>
      {(error || hint) && <div style={hintStyle(!!error)}>{error || hint}</div>}
    </div>
  );
}
