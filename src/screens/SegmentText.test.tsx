import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { SegmentText } from "./SegmentText";

function block(text: string, mixed = false): HTMLElement {
  const { container } = render(<SegmentText text={text} mixed={mixed} />);
  return container.querySelector(".seg-text") as HTMLElement;
}

describe("SegmentText", () => {
  it("draws English in the Latin font with no direction override", () => {
    const el = block("Good afternoon, and welcome.");
    expect(el.getAttribute("lang")).toBe("en");
    expect(el.hasAttribute("dir")).toBe(false);
    expect(el.style.fontFamily).toBe("var(--font-sans)");
    expect(el.querySelectorAll("span")).toHaveLength(0);
  });

  it("draws Hindi in Devanagari with the Devanagari line height", () => {
    const el = block("आज हम तीन इमारतों के आँकड़ों पर बात करेंगे।");
    expect(el.getAttribute("lang")).toBe("hi");
    expect(el.hasAttribute("dir")).toBe(false);
    expect(el.style.fontFamily).toBe("var(--font-devanagari)");
    expect(el.style.lineHeight).toBe("var(--lh-devanagari)");
  });

  it("draws Urdu right to left, aligned to the start edge, with the Nastaliq font", () => {
    const el = block("اس مطالعے میں ہم نے تین عمارتوں کی چھتوں پر لگے سولر پینلوں کا ڈیٹا دیکھا۔");
    expect(el.getAttribute("lang")).toBe("ur");
    expect(el.getAttribute("dir")).toBe("rtl");
    expect(el.style.textAlign).toBe("start");
    expect(el.style.fontFamily).toBe("var(--font-urdu)");
    expect(el.style.lineHeight).toBe("var(--lh-urdu)");
  });

  it("puts every script of a mixed Hindi sentence in its own tagged span and leaves the direction to the browser", () => {
    const el = block("तो हमारा main finding यह है कि battery storage से peak load काफ़ी कम हुआ।", true);
    expect(el.getAttribute("dir")).toBe("auto");
    expect(el.getAttribute("lang")).toBe("hi");
    const spans = [...el.querySelectorAll("span")];
    expect(spans.map((s) => s.getAttribute("lang"))).toEqual(["hi", "en", "hi", "en", "hi", "en", "hi"]);
    const latin = spans.filter((s) => s.getAttribute("lang") === "en");
    expect(latin.every((s) => s.style.fontFamily === "var(--font-sans)")).toBe(true);
    const hindi = spans.filter((s) => s.getAttribute("lang") === "hi");
    expect(hindi.every((s) => s.style.fontFamily === "var(--font-devanagari)")).toBe(true);
    expect(el.textContent).toBe("तो हमारा main finding यह है कि battery storage से peak load काफ़ी कम हुआ।");
  });

  it("does the same for Urdu with English words, in the Urdu block", () => {
    const text = "ہمارا main finding یہ ہے کہ battery storage سے peak load کافی کم ہوا۔";
    const el = block(text, true);
    expect(el.getAttribute("dir")).toBe("auto");
    expect(el.getAttribute("lang")).toBe("ur");
    expect(el.style.lineHeight).toBe("var(--lh-urdu)");
    const spans = [...el.querySelectorAll("span")];
    expect(spans.map((s) => s.getAttribute("lang"))).toEqual(["ur", "en", "ur", "en", "ur", "en", "ur"]);
    expect(
      spans
        .filter((s) => s.getAttribute("lang") === "ur")
        .every((s) => s.style.fontFamily === "var(--font-urdu)"),
    ).toBe(true);
    expect(el.textContent).toBe(text);
  });

  it("goes by the script of the text, not the language tag: Hindi speech written in Urdu script is right to left", () => {
    const el = block("یہ ایک مثال ہے۔");
    expect(el.getAttribute("lang")).toBe("ur");
    expect(el.getAttribute("dir")).toBe("rtl");
  });

  it("sets dir=auto for a segment the model marked mixed even when one script is all that came out", () => {
    const el = block("main finding", true);
    expect(el.getAttribute("dir")).toBe("auto");
    expect(el.querySelectorAll("span")).toHaveLength(0);
  });
});
