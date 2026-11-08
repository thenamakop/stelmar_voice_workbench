import type { CSSProperties } from "react";

// App component (not in the design system): the progress bar from design/mockups/mockup.css
// (.prog), built from tokens. 6 high, paper-200 track, ink-900 fill, 2 radius.
export interface ProgressBarProps {
  /** Progress from 0 to 1; values outside are clamped. */
  value: number;
  /** Accessible name, for example "Part 4 transcription". */
  label: string;
  style?: CSSProperties;
}

export function ProgressBar({ value, label, style }: ProgressBarProps) {
  const clamped = Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
  const percent = Math.round(clamped * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      style={{
        height: 6,
        borderRadius: 2,
        background: "var(--paper-200)",
        overflow: "hidden",
        ...style,
      }}
    >
      <i style={{ display: "block", height: "100%", width: `${percent}%`, background: "var(--ink-900)" }} />
    </div>
  );
}
