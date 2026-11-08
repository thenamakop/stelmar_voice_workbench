import { useState } from "react";
import type { CSSProperties, HTMLAttributes, MouseEventHandler, ReactNode } from "react";
import { Icon } from "./Icon";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/core/Tag.jsx).
export interface TagProps extends Omit<HTMLAttributes<HTMLSpanElement>, "onClick"> {
  selected?: boolean;
  onRemove?: () => void;
  onClick?: MouseEventHandler<HTMLSpanElement>;
  children?: ReactNode;
  style?: CSSProperties;
}

export function Tag({ selected, onRemove, onClick, children, style, ...rest }: TagProps) {
  const [hover, setHover] = useState(false);
  const interactive = !!onClick;
  return (
    <span
      {...rest}
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: 28,
        padding: onRemove ? px(0, 6, 0, 12) : px(0, 12),
        borderRadius: "var(--radius-pill)",
        border: px(1) + " solid " + (selected ? "var(--ink-900)" : "var(--ink-300)"),
        background: selected
          ? "var(--ink-900)"
          : hover && interactive
            ? "var(--paper-100)"
            : "var(--paper-0)",
        color: selected ? "var(--paper-50)" : "var(--ink-800)",
        fontFamily: "var(--font-ui)",
        fontSize: 13,
        cursor: interactive ? "pointer" : "default",
        userSelect: "none",
        transition: "background var(--dur-fast) var(--ease-standard)",
        ...style,
      }}
    >
      {children}
      {onRemove && (
        <span
          role="button"
          aria-label="Remove"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          style={{ display: "inline-flex", cursor: "pointer", opacity: 0.7 }}
        >
          <Icon name="x" size={14} />
        </span>
      )}
    </span>
  );
}
