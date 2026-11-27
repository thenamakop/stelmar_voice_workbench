import { describe, expect, it } from "vitest";
import {
  clock,
  format,
  gigabytes,
  initialChoice,
  istDateTime,
  languages,
  meterPercent,
  minutes,
  partFile,
  sessionLanguages,
} from "./format";

describe("recording formats", () => {
  it("formats the audio clock", () => {
    expect(clock(0)).toBe("00:00:00");
    expect(clock(2_715_999)).toBe("00:45:15");
    expect(clock(36_000_000)).toBe("10:00:00");
    expect(minutes(2_400_000)).toBe("40:00");
    expect(minutes(5_400_000)).toBe("90:00");
  });

  it("shows wall-clock times in IST", () => {
    expect(istDateTime("2027-02-08T06:30:00Z")).toBe("08 FEB 2027 · 12:00 IST");
    expect(istDateTime("2027-02-08T12:03:00+05:30")).toBe("08 FEB 2027 · 12:03 IST");
    expect(istDateTime("not a date")).toBe("");
  });

  it("maps dBFS onto the −60 to 0 meter", () => {
    expect(meterPercent(-60)).toBe(0);
    expect(meterPercent(-100)).toBe(0);
    expect(meterPercent(-12)).toBe(80);
    expect(meterPercent(0)).toBe(100);
    expect(meterPercent(3)).toBe(100);
  });

  it("formats sizes, formats, languages and part names", () => {
    expect(gigabytes(388e9)).toBe("388 GB");
    expect(format(48000, 2)).toBe("48 kHz · 2 ch");
    expect(format(44100, 1)).toBe("44.1 kHz · 1 ch");
    expect(languages(["en", "hi"])).toBe("EN · HI");
    expect(partFile(5)).toBe("part-0005.wav");
  });
});

describe("a session's languages: English plus at most one of Hindi or Urdu", () => {
  it("keeps one Hindi or Urdu as listed", () => {
    expect(sessionLanguages(["en", "hi"])).toEqual(["en", "hi"]);
    expect(sessionLanguages(["en", "ur"])).toEqual(["en", "ur"]);
    expect(sessionLanguages(["en"])).toEqual(["en"]);
  });

  it("shows Hindi, never three languages, for an old session that listed both", () => {
    expect(sessionLanguages(["en", "hi", "ur"])).toEqual(["en", "hi"]);
    expect(sessionLanguages(["ur", "en", "hi"])).toEqual(["hi", "en"]);
    expect(languages(sessionLanguages(["en", "hi", "ur"]))).toBe("EN · HI");
  });

  it("starts the dialog from the programme: Urdu is Nastaliq, both listed is Hindi, none is English and Hindi", () => {
    expect(initialChoice(["en", "ur"])).toEqual({ english: true, hindiUrdu: true, script: "nastaliq" });
    expect(initialChoice(["en", "hi", "ur"])).toEqual({
      english: true,
      hindiUrdu: true,
      script: "devanagari",
    });
    expect(initialChoice(["en"])).toEqual({ english: true, hindiUrdu: false, script: "devanagari" });
    expect(initialChoice(undefined)).toEqual({ english: true, hindiUrdu: true, script: "devanagari" });
  });
});
