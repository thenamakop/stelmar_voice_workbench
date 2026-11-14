import { expect, test, type Page } from "@playwright/test";

// Phase 4 screens at 1536x864: Queue and Record in record mode. The screenshots go to test-results by
// default; the evidence run sets WB_SCREENSHOT_DIR=docs/evidence/phase-4/screenshots (a script writes them,
// nobody edits them).
const SHOTS = process.env.WB_SCREENSHOT_DIR ?? "test-results/screenshots";

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

const CAP = "profile=present&capability=tier-c-low";

test("Queue screen with a busy day (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto(`/e2e/index.html?${CAP}&queue=busy`);
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Queue" })).toBeVisible();
  const card = page.getByLabel("Job Parallel Track Sessions I");
  await expect(card.getByText("Running", { exact: true })).toBeVisible();
  await expect(page.getByText("All transcripts ready by 19:40")).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SHOTS}/queue-busy.png` });
  expect(remote).toEqual([]);
});

test("Queue screen with the operator's pause (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto(`/e2e/index.html?${CAP}&queue=paused`);
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Queue" }).click();
  await expect(page.getByText("the operator paused the queue").first()).toBeVisible();
  await expect(page.getByRole("button", { name: /Resume All/ })).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SHOTS}/queue-paused.png` });
  expect(remote).toEqual([]);
});

test("Queue screen with no jobs (screenshot)", async ({ page }) => {
  await page.goto(`/e2e/index.html?${CAP}&queue=none`);
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Queue" }).click();
  await expect(page.getByText("No jobs", { exact: true })).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SHOTS}/queue-empty.png` });
});

test("Change Model dialog (screenshot)", async ({ page }) => {
  await page.goto(`/e2e/index.html?${CAP}&queue=busy`);
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Queue" }).click();
  await page
    .getByLabel("Job Parallel Track Sessions I")
    .getByRole("button", { name: /Change Model/ })
    .click();
  await expect(page.getByRole("dialog").getByText("Change Model")).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SHOTS}/queue-change-model.png` });
});

test("Record screen in record mode matches record-only.html (screenshot)", async ({ page }) => {
  const remote = watchRequests(page);
  await page.goto(`/e2e/index.html?${CAP}&state=recording&queue=busy`);
  await expect(page.getByRole("heading", { level: 1, name: "Parallel Track Sessions I" })).toBeVisible();
  await expect(page.getByText(/Good afternoon, and welcome/)).toBeVisible();
  const parts = page.getByLabel("Parts");
  await expect(parts.getByText("Transcribing 50%")).toBeVisible();
  await expect(parts.getByText("Transcribed").first()).toBeVisible();
  await expect(page.getByText("HI·EN").first()).toBeVisible();
  await expect(page.getByText(/اس مطالعے/)).toHaveAttribute("dir", "rtl");
  await expect(page.getByText(/Queue: 2 jobs/)).toBeVisible();
  await fontsReady(page);
  await page.screenshot({ path: `${SHOTS}/record-record-mode.png` });
  expect(remote).toEqual([]);
});
