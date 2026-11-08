import { expect, test, type Page } from "@playwright/test";

// Screenshots of the Record and System placeholders at 1536x864 for comparison with
// design/mockups/record-only.png and system-check.png, plus offline and font checks.
const SCREENSHOT_DIR = "test-results/screenshots";

/** Fails the test if the page requests anything other than the local preview server. */
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

test("first launch opens System with Run System Check disabled", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html");
  await expect(
    page.getByRole("heading", { name: "Check This Computer Before the First Recording" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Run System Check" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "System", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/system.png` });
  expect(remote).toEqual([]);
});

test("Record screen renders Inter, IBM Plex Mono, Devanagari and Nastaliq from local files", async ({
  page,
}) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present");
  await expect(page.getByRole("heading", { name: "Sessions" })).toBeVisible();
  await page.getByRole("button", { name: "Record" }).click();
  await expect(page.getByRole("heading", { name: "No Session Open" })).toBeVisible();
  await fontsReady(page);

  const loaded = await page.evaluate(async () => {
    const faces = [
      ['16px "Inter"', "Record"],
      ['16px "IBM Plex Mono"', "00:00:00"],
      ['16px "Noto Sans Devanagari"', "हिन्दी"],
      ['16px "Noto Nastaliq Urdu"', "اردو"],
    ] as const;
    const result: Record<string, boolean> = {};
    for (const [font, text] of faces) {
      await document.fonts.load(font, text);
      result[font] = document.fonts.check(font, text);
    }
    const urls = [...document.fonts].filter((f) => f.status === "loaded").map((f) => f.family);
    return { result, loadedFamilies: [...new Set(urls)].sort() };
  });
  for (const ok of Object.values(loaded.result)) expect(ok).toBe(true);
  expect(loaded.loadedFamilies).toEqual(
    expect.arrayContaining(["IBM Plex Mono", "Inter", "Noto Nastaliq Urdu", "Noto Sans Devanagari"]),
  );

  const urdu = page.getByText(/اس مطالعے/);
  await expect(urdu).toHaveAttribute("dir", "rtl");
  await page.screenshot({ path: `${SCREENSHOT_DIR}/record.png` });
  expect(remote).toEqual([]);
});

test("every screen opens from the nav", async ({ page }) => {
  await page.goto("/e2e/index.html?profile=present");
  const screens: [string, string][] = [
    ["Sessions", "Sessions"],
    ["Record", "No Session Open"],
    ["Queue", "Queue"],
    ["Review", "Transcripts"],
    ["System", "This Computer's System Check"],
    ["Settings", "Settings"],
  ];
  for (const [nav, heading] of screens) {
    await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: nav }).click();
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
    await page.screenshot({ path: `${SCREENSHOT_DIR}/${nav.toLowerCase()}-nav.png` });
  }
});
