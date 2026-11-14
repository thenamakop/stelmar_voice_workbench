import type { ReactNode } from "react";
import { IconButton } from "./IconButton";
import { px } from "./units";

// Ported from design/reference/ds-bundle.js (components/feedback/Dialog.jsx).
export interface DialogProps {
  open?: boolean;
  onClose?: () => void;
  title?: ReactNode;
  eyebrow?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  width?: number;
}

export function Dialog({ open, onClose, title, eyebrow, children, footer, width = 520 }: DialogProps) {
  if (!open) return null;
  return (
    <div
      onClick={onClose}
      style={{
        position: "fixed",
        inset: 0,
        background: "var(--scrim)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
        padding: 24,
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
        style={{
          width: "100%",
          maxWidth: width,
          background: "var(--paper-0)",
          borderRadius: "var(--radius-lg)",
          boxShadow: "var(--shadow-3)",
          borderTop: px(3) + " solid var(--ink-900)",
          overflow: "hidden",
          // A tall dialog scrolls inside the window instead of running past it (owner, 14 Nov 2026): the header
          // and the footer stay, the body scrolls. Nothing changes while it fits.
          display: "flex",
          flexDirection: "column",
          maxHeight: `calc(100vh - ${px(48)})`,
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "flex-start",
            gap: 16,
            padding: px(20, 24, 0),
            flexShrink: 0,
          }}
        >
          <div>
            {eyebrow && (
              <div
                style={{
                  fontFamily: "var(--font-ui)",
                  fontSize: 12,
                  fontWeight: 600,
                  letterSpacing: "0.08em",
                  textTransform: "uppercase",
                  color: "var(--ink-600)",
                  marginBottom: 6,
                }}
              >
                {eyebrow}
              </div>
            )}
            <div
              style={{
                fontFamily: "var(--font-heading)",
                fontSize: 24,
                lineHeight: px(30),
                fontWeight: 700,
                color: "var(--ink-900)",
              }}
            >
              {title}
            </div>
          </div>
          {onClose && <IconButton icon="x" label="Close" size="sm" onClick={onClose} />}
        </div>
        <div
          style={{
            padding: px(12, 24, 24),
            overflowY: "auto",
            minHeight: 0,
            fontFamily: "var(--font-ui)",
            fontSize: 15,
            lineHeight: px(24),
            color: "var(--ink-800)",
          }}
        >
          {children}
        </div>
        {footer && (
          <div
            style={{
              display: "flex",
              justifyContent: "flex-end",
              gap: 8,
              padding: px(14, 24),
              background: "var(--paper-50)",
              borderTop: px(1) + " solid var(--ink-200)",
              flexShrink: 0,
            }}
          >
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}
