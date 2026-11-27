import { describe, expect, it } from "vitest";
import { blockScript, scriptOf, splitRuns } from "./scriptRuns";

const cp = (s: string): number => s.codePointAt(0) ?? 0;
const join = (text: string) =>
  splitRuns(text)
    .map((r) => r.text)
    .join("");

describe("scriptOf", () => {
  it("tells the three scripts apart and leaves spaces, digits and punctuation neutral", () => {
    expect(scriptOf(cp("a"))).toBe("latin");
    expect(scriptOf(cp("Z"))).toBe("latin");
    expect(scriptOf(cp("é"))).toBe("latin");
    expect(scriptOf(cp("क"))).toBe("devanagari");
    expect(scriptOf(cp("।"))).toBe("devanagari");
    expect(scriptOf(cp("ا"))).toBe("arabic");
    expect(scriptOf(cp("ے"))).toBe("arabic");
    for (const ch of [" ", "5", ",", ".", "-", "\u200c", "\u200d", "×", "÷"]) {
      expect(scriptOf(cp(ch))).toBeNull();
    }
  });
});

describe("splitRuns", () => {
  it("gives one run for a single-script segment", () => {
    expect(splitRuns("Good afternoon, and welcome.")).toEqual([
      { script: "latin", lang: "en", text: "Good afternoon, and welcome." },
    ]);
    expect(splitRuns("आज हम तीन इमारतों पर बात करेंगे।")).toHaveLength(1);
    expect(splitRuns("اس مطالعے میں ہم نے ڈیٹا دیکھا۔")).toHaveLength(1);
  });

  it("keeps each word of a Hindi sentence with English words in its own script", () => {
    const runs = splitRuns("तो हमारा main finding यह है कि battery storage से peak load काफ़ी कम हुआ।");
    expect(runs.map((r) => r.lang)).toEqual(["hi", "en", "hi", "en", "hi", "en", "hi"]);
    expect(runs[1]?.text).toBe("main finding ");
    expect(runs[3]?.text).toBe("battery storage ");
    expect(runs[5]?.text).toBe("peak load ");
  });

  it("splits Urdu with English words the same way", () => {
    const runs = splitRuns("ہمارا main finding یہ ہے کہ battery storage سے peak load کافی کم ہوا۔");
    expect(runs.map((r) => r.script)).toEqual([
      "arabic",
      "latin",
      "arabic",
      "latin",
      "arabic",
      "latin",
      "arabic",
    ]);
    expect(runs.every((r) => r.lang === (r.script === "arabic" ? "ur" : "en"))).toBe(true);
  });

  it("never loses or reorders a character", () => {
    for (const text of [
      "तो हमारा main finding यह है कि battery storage से peak load काफ़ी कम हुआ।",
      "Next, यह graph बताता है कि सर्दियों में output कितना गिरता है।",
      "ہمارا main finding یہ ہے",
      "  leading space and 12:30 digits, then क्या",
      "ग़ज़ल with a zero\u200bwidth and a joiner क\u200dष",
      "",
      "...",
      "1234",
    ]) {
      expect(join(text)).toBe(text);
    }
  });

  it("puts leading neutrals in the first run and keeps joiners and combining marks with their letters", () => {
    expect(splitRuns("  क्या")[0]?.text).toBe("  क्या");
    const runs = splitRuns("e\u0301 and क\u200dष");
    expect(runs.map((r) => r.lang)).toEqual(["en", "hi"]);
    expect(runs[0]?.text).toBe("e\u0301 and ");
    expect(runs[1]?.text).toBe("क\u200dष");
  });

  it("returns one Latin run for text with no letters, and nothing for an empty string", () => {
    expect(splitRuns("12:30 - 13:00")).toEqual([{ script: "latin", lang: "en", text: "12:30 - 13:00" }]);
    expect(splitRuns("")).toEqual([]);
  });

  it("keeps Devanagari and Arabic-Indic digits with their own script and ASCII digits with the run before", () => {
    const runs = splitRuns("साल २०२६ and 2026");
    expect(runs.map((r) => r.lang)).toEqual(["hi", "en"]);
    expect(runs[0]?.text).toBe("साल २०२६ ");
  });
});

describe("blockScript", () => {
  it("is Latin for English, and the Indic or Arabic script when the text has any", () => {
    expect(blockScript("Good afternoon.")).toBe("latin");
    expect(blockScript("आज हम बात करेंगे।")).toBe("devanagari");
    expect(blockScript("اس مطالعے میں")).toBe("arabic");
    expect(blockScript("तो हमारा main finding यह है कि battery storage से peak load काफ़ी कम हुआ।")).toBe(
      "devanagari",
    );
    expect(blockScript("ہمارا main finding یہ ہے")).toBe("arabic");
    expect(blockScript("main finding क")).toBe("devanagari");
    expect(blockScript("")).toBe("latin");
  });
});
