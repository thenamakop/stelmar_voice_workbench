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

test("first launch opens System with Run Quick Check available", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html");
  await expect(
    page.getByRole("heading", { name: "Check This Computer Before the First Recording" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Run Quick Check" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "System", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/system.png` });
  expect(remote).toEqual([]);
});

test("System screen scrolled to the engines and models sections (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html");
  const models = page.getByText("Models", { exact: true }).first();
  await expect(page.getByText("Speech engines", { exact: true })).toBeVisible();
  await expect(models).toBeVisible();
  await expect(page.getByText("READY").first()).toBeVisible();
  await models.scrollIntoViewIfNeeded();
  await page.evaluate(() => {
    const el = [...document.querySelectorAll("*")].find(
      (e) => e.scrollHeight > e.clientHeight + 40 && getComputedStyle(e).overflowY !== "visible",
    );
    if (el) el.scrollTop = el.scrollHeight;
  });
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/system-engines-models.png` });
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

test("Record screen with every speech engine renamed away shows the banner (screenshot)", async ({
  page,
}) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present&engines=none&state=idle");
  await page.getByRole("button", { name: "Record", exact: true }).click();
  const banner = page.getByRole("status", { name: "Speech engine" });
  await expect(banner).toContainText("No speech engine found. Recording is not affected.");
  // The banner must not push the header controls: Start Session stays where it is and stays enabled.
  await expect(page.getByRole("button", { name: /Start Session/ })).toBeEnabled();
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/record-no-engine.png` });
  expect(remote).toEqual([]);
});

test("Record screen with the engines present has no banner", async ({ page }) => {
  await page.goto("/e2e/index.html?profile=present&state=idle");
  await page.getByRole("button", { name: "Record", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "No Session Open" })).toBeVisible();
  await expect(page.getByRole("status", { name: "Speech engine" })).toHaveText("");
});

test("Record screen while recording matches record-only.html (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present&state=recording");
  await expect(page.getByRole("heading", { level: 1, name: "Parallel Track Sessions I" })).toBeVisible();
  await expect(page.getByLabel("Recorded time")).toHaveText("00:45:15");
  await expect(page.getByRole("button", { name: "Stop Session" })).toBeVisible();
  await expect(page.getByText("In sync")).toBeVisible();
  await expect(page.getByLabel("Parts").getByText("Recording")).toBeVisible();
  // The level event arrives on the channel shortly after subscribing.
  await expect(page.getByRole("meter", { name: "Input level" })).toHaveAttribute("aria-valuenow", "-21");
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/record-recording.png` });
  expect(remote).toEqual([]);
});

test("input lost shows the banner and keeps the session open (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present&state=input_lost");
  await expect(page.getByRole("alert")).toContainText("Input lost");
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/record-input-lost.png` });
  expect(remote).toEqual([]);
});

test("crash recovery offers Resume and Close (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present&state=recovery");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Resume This Session?")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Resume Recording" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Close Session" })).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/recovery-dialog.png` });
  await dialog.getByRole("button", { name: "Resume Recording" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Parallel Track Sessions I" })).toBeVisible();
  expect(remote).toEqual([]);
});

test("Start Session dialog (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present");
  await page.getByRole("button", { name: "Record" }).click();
  await page.getByRole("button", { name: "Start Session" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SCREENSHOT_DIR}/start-dialog-phase1.png` });
  expect(remote).toEqual([]);
});

test("every screen opens from the nav", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto("/e2e/index.html?profile=present");
  const screens: [string, string][] = [
    ["Sessions", "Sessions"],
    ["Record", "No Session Open"],
    ["Queue", "Queue"],
    ["Review", "Transcripts"],
    ["System", "Record Now, Transcribe in the Background"],
    ["Settings", "Settings"],
  ];
  for (const [nav, heading] of screens) {
    await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: nav }).click();
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
    await page.screenshot({ path: `${SCREENSHOT_DIR}/${nav.toLowerCase()}-nav.png` });
  }
  expect(remote).toEqual([]);
});
