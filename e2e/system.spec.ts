import { expect, test, type Page } from "@playwright/test";

// The System screen and the Start Session dialog against the fixture profiles of
// crates/wb-capability/fixtures (served by the mock bridge, src/test/e2e-entry.ts), at 1536x864 for
// comparison with design/mockups/system-check.png and start-session.png. The screenshots go to
// WB_SCREENSHOT_DIR (docs/evidence/phase-3/screenshots when the gate script runs this).
const SHOTS = process.env.WB_SCREENSHOT_DIR ?? "test-results/screenshots";

function watchRequests(page: Page): string[] {
  const remote: string[] = [];
  page.on("request", (req) => {
    const url = new URL(req.url());
    if (url.hostname !== "localhost" && url.protocol !== "data:") remote.push(req.url());
  });
  return remote;
}

async function ready(page: Page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
}

async function openSystem(page: Page, capability: string, extra = "") {
  await page.goto(`/e2e/index.html?capability=${capability}${extra}`);
  await page.getByRole("button", { name: "System", exact: true }).click();
}

async function openDialog(page: Page, capability: string) {
  await page.goto(`/e2e/index.html?capability=${capability}`);
  await page.getByRole("button", { name: "Record", exact: true }).click();
  const start = page.getByRole("button", { name: "Start Session" });
  await expect(start).toBeEnabled();
  await start.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  return dialog;
}

const SYSTEM_HEADING: Record<string, string> = {
  "tier-g": "Record Now, Live Captions Available",
  "tier-c-low": "Record Now, Transcribe in the Background",
  "tier-c-mid-hybrid": "Record Now, Transcribe in the Background",
  "tier-c-plus": "Record Now, Near-Live Text Available",
  "tier-c-cpu-only": "Record Now, Transcribe in the Background",
  "tier-r": "This Computer Records Only",
  "slow-igpu": "Record Now, Transcribe in the Background",
};

for (const [name, heading] of Object.entries(SYSTEM_HEADING)) {
  test(`System screen, profile ${name}`, async ({ page }) => {
    const remote = watchRequests(page);
    await openSystem(page, name);
    await expect(page.getByRole("heading", { name: heading })).toBeVisible();
    await expect(page.getByText("Hardware", { exact: true }).first()).toBeVisible();
    await ready(page);
    if (name === "tier-c-low" || name === "tier-g" || name === "tier-r") {
      await page.screenshot({ path: `${SHOTS}/system-${name}.png` });
    }
    expect(remote).toEqual([]);
  });
}

test("System screen shows the fingerprint banner, and the Emulated badge only on an emulated profile", async ({ page }) => {
  await openSystem(page, "tier-c-low", "&fingerprint=changed");
  await expect(page.getByText(/has changed since the last check/)).toBeVisible();
  await expect(page.getByText("Emulated · emu-low")).toBeVisible();
  await openSystem(page, "tier-g");
  await expect(page.getByText(/Emulated/)).toHaveCount(0);
  await openSystem(page, "tier-c-low", "&fingerprint=changed");
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/system-fingerprint-banner.png` });
});

test("a safeguard override asks first", async ({ page }) => {
  await openSystem(page, "tier-c-low");
  await page.getByText("Pause on battery power", { exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/runs on its battery/)).toBeVisible();
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/system-override-dialog.png` });
  await dialog.getByRole("button", { name: "Keep the current setting" }).click();
  await expect(dialog).toHaveCount(0);
});

for (const name of ["tier-c-low", "tier-c-cpu-only", "slow-igpu", "tier-r", "tier-c-mid-hybrid"]) {
  test(`Start Session dialog on ${name}: Record + post-process by default, live off with its reason`, async ({ page }) => {
    const dialog = await openDialog(page, name);
    await expect(dialog.getByRole("radio", { name: /Record \+ post-process/ })).toBeChecked();
    await expect(dialog.getByRole("radio", { name: /Live captions \+ post-process/ })).toBeDisabled();
    await expect(dialog.getByRole("checkbox", { name: "English" })).toBeChecked();
    await expect(dialog.getByRole("checkbox", { name: "Hindi / Urdu" })).toBeChecked();
    await expect(dialog.getByRole("radio", { name: /Devanagari/ })).toBeChecked();
    await ready(page);
    if (name === "tier-c-low") await page.screenshot({ path: `${SHOTS}/start-session-tier-c-low.png` });
  });
}

test("Start Session dialog on the tier G profile offers live but starts in Record + post-process", async ({ page }) => {
  const dialog = await openDialog(page, "tier-g");
  await expect(dialog.getByRole("radio", { name: /Live captions \+ post-process/ })).toBeEnabled();
  await expect(dialog.getByRole("radio", { name: /Record \+ post-process/ })).toBeChecked();
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/start-session-tier-g.png` });
});

test("Start Session dialog: the script choice appears with Hindi / Urdu and Urdu is right to left", async ({ page }) => {
  const dialog = await openDialog(page, "tier-c-low");
  await dialog.getByText(/Nastaliq/).first().click();
  await expect(dialog.locator('[lang="ur"][dir="rtl"]')).toBeVisible();
  await dialog.getByText("Hindi / Urdu", { exact: true }).click();
  await expect(dialog.getByRole("radio", { name: /Devanagari/ })).toHaveCount(0);
  await expect(dialog.getByText(/Locked to English/)).toBeVisible();
});

test("Start Session dialog without a saved profile says live needs the system check", async ({ page }) => {
  await page.goto("/e2e/index.html?state=idle");
  await page.getByRole("button", { name: "Record", exact: true }).click();
  const start = page.getByRole("button", { name: "Start Session" });
  await expect(start).toBeEnabled();
  await start.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(/Not offered until the system check has run/)).toBeVisible();
  await expect(dialog.getByRole("radio", { name: /Live captions \+ post-process/ })).toBeDisabled();
});
