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

async function openDialog(page: Page, capability: string, extra = "") {
  await page.goto(`/e2e/index.html?capability=${capability}${extra}`);
  await page.getByRole("button", { name: "Record", exact: true }).click();
  const start = page.getByRole("button", { name: "Start Session" });
  await expect(start).toBeEnabled();
  await start.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  return dialog;
}

const SYSTEM_HEADING: Record<string, string> = {
  "tier-g": "Record Now, Live Captions for English",
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

test("System screen shows the fingerprint banner, and the Emulated badge only on an emulated profile", async ({
  page,
}) => {
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
  await dialog.getByRole("button", { name: "Keep the Current Setting" }).click();
  await expect(dialog).toHaveCount(0);
});

for (const name of ["tier-c-low", "tier-c-cpu-only", "slow-igpu", "tier-r", "tier-c-mid-hybrid"]) {
  test(`Start Session dialog on ${name}: Record + post-process by default, live off with its reason`, async ({
    page,
  }) => {
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

test("Start Session dialog on the tier G profile: live for English only, starts in Record + post-process", async ({
  page,
}) => {
  const dialog = await openDialog(page, "tier-g");
  const live = dialog.getByRole("radio", { name: /Live captions \+ post-process/ });
  await expect(dialog.getByRole("radio", { name: /Record \+ post-process/ })).toBeChecked();
  // English + Hindi / Urdu (the default): no live model has been checked for Hindi or Urdu yet, so live is off with its reason.
  await expect(live).toBeDisabled();
  await expect(dialog.getByText(/checked for accuracy/)).toBeVisible();
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/start-session-tier-g.png` });
  // English only: the graphics card's live model has been measured for English.
  await dialog.getByText("Hindi / Urdu", { exact: true }).click();
  await expect(live).toBeEnabled();
  await expect(dialog.getByRole("radio", { name: /Record \+ post-process/ })).toBeChecked();
});

test("Start Session dialog while a system check runs: Start stays enabled and the note says it stops the check", async ({
  page,
}) => {
  const dialog = await openDialog(page, "tier-c-low", "&check=running");
  await expect(dialog.getByText(/Starting the recording stops the check/)).toBeVisible();
  await dialog.getByLabel(/Session title/).fill("Keynote");
  await expect(dialog.getByRole("button", { name: "Start Recording" })).toBeEnabled();
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/start-session-check-running.png` });
});

test("Record screen side card shows the profile's devices, speed and threads (screenshot)", async ({
  page,
}) => {
  await page.goto("/e2e/index.html?capability=tier-g");
  await page.getByRole("button", { name: "Record", exact: true }).click();
  const card = page.locator("aside").filter({ hasText: "Transcription" });
  await expect(card.getByText("RTX 4070")).toBeVisible();
  await expect(card.getByText("Speed", { exact: true })).toBeVisible();
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/record-tier-g.png` });
});

test("Start Session dialog: the script choice appears with Hindi / Urdu and Urdu is right to left", async ({
  page,
}) => {
  const dialog = await openDialog(page, "tier-c-low");
  await dialog
    .getByText(/Nastaliq/)
    .first()
    .click();
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

test("System screen lists the complete check and a partial one kept apart", async ({ page }) => {
  await openSystem(page, "tier-g", "&partial=tier-c-low");
  await expect(page.getByText("Saved checks", { exact: true })).toBeVisible();
  await expect(page.getByText("Complete check", { exact: true })).toBeVisible();
  await expect(page.getByText("Partial check", { exact: true })).toBeVisible();
  await expect(page.getByText("Kept apart; the complete check stays in use")).toBeVisible();
  await expect(page.getByText("A partial check is kept apart")).toBeVisible();
  await page.getByText("Saved checks", { exact: true }).scrollIntoViewIfNeeded();
  await page.getByText("Kept apart; the complete check stays in use").scrollIntoViewIfNeeded();
  await ready(page);
  await page.screenshot({ path: `${SHOTS}/system-saved-checks.png` });
});

test("after a fingerprint change the System screen and the dialog say live and processing while recording are off", async ({
  page,
}) => {
  await openSystem(page, "tier-g", "&fingerprint=changed");
  await expect(page.getByText(/live captions and processing while recording are off/)).toBeVisible();
  const dialog = await openDialog(page, "tier-g", "&fingerprint=changed");
  // English only would be offered on tier G; after the change nothing is.
  await dialog.getByText("Hindi / Urdu", { exact: true }).click();
  await expect(dialog.getByText(/This computer has changed since the check/).first()).toBeVisible();
  await expect(dialog.getByText("Record + post-process", { exact: false }).first()).toBeVisible();
});

test("the Start Session dialog scrolls inside a 1366x768 window and keeps Start in reach", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  const dialog = await openDialog(page, "tier-c-low", "&check=running");
  const box = await dialog.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height).toBeLessThanOrEqual(768);
  // The footer (Cancel, Start) stays inside the window; the body is what scrolls.
  const start = dialog.getByRole("button", { name: /Start/ }).last();
  await expect(start).toBeVisible();
  const startBox = await start.boundingBox();
  expect(startBox!.y + startBox!.height).toBeLessThanOrEqual(768);
  const scrolls = await dialog.evaluate((el) => {
    const body = Array.from(el.querySelectorAll<HTMLElement>("div")).find(
      (d) => getComputedStyle(d).overflowY === "auto",
    );
    return body ? body.scrollHeight > body.clientHeight : false;
  });
  expect(scrolls).toBe(true);
  await page.screenshot({ path: `${SHOTS}/start-session-1366x768.png` });
});

test("the Start Session dialog needs no scrolling at 1536x864 with a saved profile", async ({ page }) => {
  const dialog = await openDialog(page, "tier-c-low");
  const box = await dialog.boundingBox();
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height).toBeLessThanOrEqual(864);
});
