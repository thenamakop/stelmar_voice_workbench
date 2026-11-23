import { expect, test, type Page } from "@playwright/test";

// Phase 5, step 1: Hindi, Urdu and mixed text at 1536x864. The screenshots go to test-results by default; the
// evidence run sets WB_SCREENSHOT_DIR=docs/evidence/phase-5/screenshots (a script writes them, nobody edits
// them).
const SHOTS = process.env.WB_SCREENSHOT_DIR ?? "test-results/screenshots";
const CAP = "profile=present&capability=tier-c-low";

function watchRequests(page: Page): string[] {
  const remote: string[] = [];
  page.on("request", (req) => {
    const url = new URL(req.url());
    if (url.hostname !== "localhost" && url.protocol !== "data:") remote.push(req.url());
  });
  return remote;
}

async function fontsReady(page: Page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
}

test("Record screen: every word in its own script and font, Urdu right to left (screenshot)", async ({
  page,
}) => {
  const remote = watchRequests(page);
  await page.goto(`/e2e/index.html?${CAP}&state=recording&queue=busy`);
  await expect(page.getByText(/Good afternoon, and welcome/)).toBeVisible();

  // The local fonts are loaded from the app, not a CDN: Devanagari and Nastaliq must be in document.fonts.
  await fontsReady(page);
  const loaded = await page.evaluate(() =>
    [...document.fonts].filter((f) => f.status === "loaded").map((f) => f.family),
  );
  expect(loaded.join(" ")).toContain("Noto Sans Devanagari");
  expect(loaded.join(" ")).toContain("Noto Nastaliq Urdu");

  // Hindi with English words: Devanagari runs in the Devanagari font, English runs in the Latin font.
  const hindiMixed = page.locator(".seg-text", { hasText: "battery storage से" }).first();
  await expect(hindiMixed).toHaveAttribute("dir", "auto");
  const runs = await hindiMixed.locator("span").evaluateAll((spans) =>
    spans.map((s) => ({
      lang: s.getAttribute("lang"),
      font: getComputedStyle(s).fontFamily,
      text: s.textContent,
    })),
  );
  expect(runs.map((r) => r.lang)).toEqual(["hi", "en", "hi", "en", "hi", "en", "hi"]);
  for (const r of runs) {
    if (r.lang === "hi") expect(r.font).toContain("Noto Sans Devanagari");
    if (r.lang === "en") expect(r.font).toContain("Inter");
  }

  // Urdu: right to left, the Urdu block starts at the right edge of its cell.
  const urdu = page.locator(".seg-text", { hasText: "اس مطالعے" });
  await expect(urdu).toHaveAttribute("lang", "ur");
  await expect(urdu).toHaveAttribute("dir", "rtl");
  expect(await urdu.evaluate((el) => getComputedStyle(el).direction)).toBe("rtl");
  expect(await urdu.evaluate((el) => getComputedStyle(el).fontFamily)).toContain("Noto Nastaliq Urdu");
  const box = await urdu.boundingBox();
  const textRight = await urdu.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    return range.getBoundingClientRect().right;
  });
  expect(box).not.toBeNull();
  expect(Math.abs(textRight - (box?.x ?? 0) - (box?.width ?? 0))).toBeLessThan(2);

  // Urdu with English words: automatic direction, Urdu runs in Nastaliq, English runs in the Latin font.
  const urduMixed = page.locator(".seg-text", { hasText: "battery storage سے" });
  await expect(urduMixed).toHaveAttribute("dir", "auto");
  const urduRuns = await urduMixed
    .locator("span")
    .evaluateAll((spans) =>
      spans.map((s) => ({ lang: s.getAttribute("lang"), font: getComputedStyle(s).fontFamily })),
    );
  expect(urduRuns.map((r) => r.lang)).toEqual(["ur", "en", "ur", "en", "ur", "en", "ur"]);
  expect(urduRuns.filter((r) => r.lang === "ur").every((r) => r.font.includes("Noto Nastaliq Urdu"))).toBe(
    true,
  );
  expect(urduRuns.filter((r) => r.lang === "en").every((r) => r.font.includes("Inter"))).toBe(true);

  await page.screenshot({ path: `${SHOTS}/record-languages.png` });
  expect(remote).toEqual([]);
});

test("editable field: Urdu and mixed text type, select, copy and move the caret by logical order", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const remote = watchRequests(page);
  await page.goto(`/e2e/index.html?${CAP}`);
  await page.getByRole("button", { name: "Record" }).click();
  await page.getByRole("button", { name: "Start Session" }).click();
  const title = page.getByLabel("Session title");
  const hall = page.getByLabel("Hall");
  await expect(title).toHaveAttribute("dir", "auto");

  // Urdu: typing keeps the characters in logical order; in a right-to-left field Home is the logical start, and the
  // left arrow moves the caret forward through the text (to the visual left).
  const urdu = "ہمارا سیشن";
  await title.fill("");
  await title.pressSequentially(urdu);
  await expect(title).toHaveValue(urdu);
  await title.press("Home");
  const at = () => title.evaluate((el) => (el as HTMLInputElement).selectionStart);
  expect(await at()).toBe(0);
  await title.press("ArrowLeft");
  expect(await at()).toBe(1);
  await title.press("ArrowRight");
  expect(await at()).toBe(0);
  await title.press("End");
  expect(await at()).toBe(urdu.length);

  // Select all, copy, paste into the other field: the text comes back unchanged.
  await title.press("Control+A");
  expect(
    await title.evaluate((el) => [
      (el as HTMLInputElement).selectionStart,
      (el as HTMLInputElement).selectionEnd,
    ]),
  ).toEqual([0, urdu.length]);
  await title.press("Control+C");
  await hall.click();
  await hall.press("Control+V");
  await expect(hall).toHaveValue(urdu);

  // Mixed text: Devanagari, English and Urdu in one field, typed and read back in logical order.
  const mixed = "मेरा main finding ہمارا";
  await hall.fill("");
  await hall.pressSequentially(mixed);
  await expect(hall).toHaveValue(mixed);
  await hall.press("Control+A");
  await hall.press("Control+C");
  await title.click();
  await title.press("Control+A");
  await title.press("Control+V");
  await expect(title).toHaveValue(mixed);
  // Shift + arrow from the start selects whole characters.
  await title.press("Home");
  await title.press("Shift+ArrowRight");
  await title.press("Shift+ArrowRight");
  const selected = await title.evaluate((el) => {
    const input = el as HTMLInputElement;
    return input.value.slice(input.selectionStart ?? 0, input.selectionEnd ?? 0).length;
  });
  // The caret moves by user-perceived character, not by code unit: मे and रा are two characters of two code units each.
  expect(selected).toBe(4);

  await fontsReady(page);
  await page.screenshot({ path: `${SHOTS}/start-dialog-mixed-text.png` });
  expect(remote).toEqual([]);
});
