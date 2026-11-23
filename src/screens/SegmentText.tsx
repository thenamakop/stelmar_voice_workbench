import type { CSSProperties } from "react";
import { px } from "../ds/units";
import { blockScript, splitRuns } from "../text/scriptRuns";
import type { Script } from "../text/scriptRuns";

// One transcript segment, drawn the way PLAN.md section 9 asks: every word in its own script and font
// (Hindi in Devanagari, English in Latin, Urdu in right-to-left Nastaliq), each run tagged with its
// language, and a segment that holds more than one script left to the browser's own direction rule
// (`dir="auto"`). The script of the text decides, not the language tag of the segment: the organiser may
// choose Devanagari or Urdu script for Hindi and Urdu speech.

const FONT: Record<Script, string> = {
  latin: "var(--font-sans)",
  devanagari: "var(--font-devanagari)",
  arabic: "var(--font-urdu)",
};

const BLOCK: Record<Script, CSSProperties> = {
  latin: { fontSize: 17, lineHeight: px(28) },
  devanagari: { fontSize: 18, lineHeight: "var(--lh-devanagari)" },
  arabic: { fontSize: 19, lineHeight: "var(--lh-urdu)" },
};

const LANG: Record<Script, string> = { latin: "en", devanagari: "hi", arabic: "ur" };

export function SegmentText({ text, mixed = false }: { text: string; mixed?: boolean }) {
  const runs = splitRuns(text);
  const script = blockScript(text);
  const scripts = new Set(runs.map((r) => r.script));
  const several = scripts.size > 1;
  // Several scripts, or a segment the model marked as mixed: the first strong letter decides. One right-to-left
  // script: right to left, aligned to the start edge (the right). Anything else: the page's own direction.
  const dir = several || mixed ? "auto" : script === "arabic" ? "rtl" : undefined;
  return (
    <div
      className="seg-text"
      lang={LANG[script]}
      dir={dir}
      style={{
        ...BLOCK[script],
        fontFamily: FONT[script],
        color: "var(--text-body)",
        textAlign: "start",
      }}
    >
      {several
        ? runs.map((r, i) => (
            <span key={i} lang={r.lang} style={{ fontFamily: FONT[r.script], lineHeight: "inherit" }}>
              {r.text}
            </span>
          ))
        : text}
    </div>
  );
}
