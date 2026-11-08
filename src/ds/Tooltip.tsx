import { useState } from "react";
import type { ReactNode } from "react";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/feedback/Tooltip.jsx).
export interface TooltipProps {
  content: ReactNode;
  placement?: "top" | "bottom";
  children?: ReactNode;
}

export function Tooltip({ content, placement = "top", children }: TooltipProps) {
  const [show, setShow] = useState(false);
  const offset = "calc(100% + " + px(6) + ")";
  const pos = placement === "bottom" ? { top: offset } : { bottom: offset };
  return (
    <span
      style={{ position: "relative", display: "inline-flex" }}
      onMouseEnter={() => setShow(true)}
      onMouseLeave={() => setShow(false)}
      onFocus={() => setShow(true)}
      onBlur={() => setShow(false)}
    >
      {children}
      <span
        role="tooltip"
        style={{
          position: "absolute",
          left: "50%",
          transform: "translateX(-50%)",
          ...pos,
          whiteSpace: "nowrap",
          pointerEvents: "none",
          background: "var(--ink-900)",
          color: "var(--paper-50)",
          fontFamily: "var(--font-ui)",
          fontSize: 12,
          lineHeight: px(16),
          padding: px(5, 8),
          borderRadius: "var(--radius-xs)",
          opacity: show ? 1 : 0,
          transition: "opacity var(--dur-fast) var(--ease-standard)",
          zIndex: 50,
        }}
      >
        {content}
      </span>
    </span>
  );
}
