// Splits transcript text into runs of one script each, so every word is shown in its own script and font
// (AGENTS.md rule 6: Hindi in Devanagari, English in Latin, Urdu in right-to-left Nastaliq). The language
// tag of a segment says what the speaker said; the script of the text says how to draw it, because the
// organiser may choose Devanagari or Urdu script for Hindi and Urdu speech, and a mixed segment holds more
// than one script.

export type Script = "latin" | "devanagari" | "arabic";

export interface ScriptRun {
  script: Script;
  /** BCP 47 language tag for the run: `en`, `hi` or `ur`. */
  lang: "en" | "hi" | "ur";
  text: string;
}

const LANG_OF: Record<Script, ScriptRun["lang"]> = { latin: "en", devanagari: "hi", arabic: "ur" };

/** The script of one code point, or null for spaces, punctuation, digits and joiners (neutral). */
export function scriptOf(cp: number): Script | null {
  // Devanagari and Devanagari Extended; the danda marks (U+0964, U+0965) belong to Devanagari text.
  if ((cp >= 0x0900 && cp <= 0x097f) || (cp >= 0xa8e0 && cp <= 0xa8ff)) return "devanagari";
  // Arabic, Arabic Supplement, Extended-A and the presentation forms Urdu text can use.
  if (
    (cp >= 0x0600 && cp <= 0x06ff) ||
    (cp >= 0x0750 && cp <= 0x077f) ||
    (cp >= 0x08a0 && cp <= 0x08ff) ||
    (cp >= 0xfb50 && cp <= 0xfdff) ||
    (cp >= 0xfe70 && cp <= 0xfeff)
  ) {
    return "arabic";
  }
  // Latin letters, Latin-1 and Latin Extended letters, but not the Latin-1 punctuation and signs.
  if (
    (cp >= 0x41 && cp <= 0x5a) ||
    (cp >= 0x61 && cp <= 0x7a) ||
    (cp >= 0xc0 && cp <= 0x24f && cp !== 0xd7 && cp !== 0xf7) ||
    (cp >= 0x1e00 && cp <= 0x1eff)
  ) {
    return "latin";
  }
  return null;
}

function isCombining(cp: number): boolean {
  return cp >= 0x0300 && cp <= 0x036f;
}

/**
 * Runs of one script each, in order. Neutral characters (spaces, punctuation, ASCII digits, joiners and
 * combining marks) stay with the run before them; neutrals at the very start go to the first run. Text
 * without any letter comes back as one Latin run. Joining the runs gives the input back.
 */
export function splitRuns(text: string): ScriptRun[] {
  const runs: ScriptRun[] = [];
  let lead = "";
  for (const ch of text) {
    const cp = ch.codePointAt(0) ?? 0;
    const script = isCombining(cp) ? null : scriptOf(cp);
    const last = runs[runs.length - 1];
    if (script === null) {
      if (last) last.text += ch;
      else lead += ch;
    } else if (last && last.script === script) {
      last.text += ch;
    } else {
      runs.push({ script, lang: LANG_OF[script], text: lead + ch });
      lead = "";
    }
  }
  if (runs.length === 0) return text === "" ? [] : [{ script: "latin", lang: "en", text }];
  return runs;
}

/**
 * The script that sets a block's font size and line height: the Indic or Arabic script with more letters
 * when the text has any (their line boxes are taller than Latin), otherwise Latin. Devanagari wins a tie.
 */
export function blockScript(text: string): Script {
  let devanagari = 0;
  let arabic = 0;
  for (const ch of text) {
    const s = scriptOf(ch.codePointAt(0) ?? 0);
    if (s === "devanagari") devanagari += 1;
    else if (s === "arabic") arabic += 1;
  }
  if (devanagari === 0 && arabic === 0) return "latin";
  return arabic > devanagari ? "arabic" : "devanagari";
}
