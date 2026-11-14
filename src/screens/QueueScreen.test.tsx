import { describe, expect, it } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { mockIPC } from "@tauri-apps/api/mocks";
import { App } from "../App";
import { bridge } from "../test/mockBridge";
import type { MockOptions } from "../test/mockBridge";

async function openQueue(opts: MockOptions) {
  mockIPC(bridge({ profile: true, capability: "tier-c-low", ...opts }));
  render(<App />);
  await screen.findByRole("heading", { name: "Sessions" });
  fireEvent.click(screen.getByRole("button", { name: "Queue" }));
}

describe("Queue screen", () => {
  it("says when nothing is queued", async () => {
    await openQueue({ queue: "none" });
    expect(await screen.findByText("No jobs")).toBeTruthy();
    expect(screen.getByText("Nothing is waiting to be transcribed")).toBeTruthy();
    expect((screen.getByRole("button", { name: /Pause All/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows a row for each job with its models, progress and the ready time", async () => {
    await openQueue({ queue: "busy" });
    const card = await screen.findByLabelText("Job Parallel Track Sessions I");
    expect(within(card).getByText("Running")).toBeTruthy();
    expect(within(card).getByText("English: small · Hindi: turbo-q5_0")).toBeTruthy();
    expect(within(card).getByRole("progressbar").getAttribute("aria-valuenow")).toBe("55");
    expect(within(card).getByText(/5 of 9 units · 1 running/)).toBeTruthy();
    expect(screen.getByText("All transcripts ready by 19:40")).toBeTruthy();
    expect(screen.getByText("Night pass")).toBeTruthy();
    expect(screen.getByText("Finished")).toBeTruthy();
  });

  it("sends the job's own id with Pause and Move Up", async () => {
    const calls: string[] = [];
    await openQueue({ queue: "busy", calls });
    const card = await screen.findByLabelText("Job Parallel Track Sessions I");
    fireEvent.click(within(card).getByRole("button", { name: /Pause/ }));
    fireEvent.click(within(card).getByRole("button", { name: /Move Up/ }));
    await waitFor(() => expect(calls).toContain("queue_pause"));
    expect(calls).toContain("queue_move_up");
  });

  it("shows why processing is held and offers Resume All", async () => {
    await openQueue({ queue: "paused" });
    expect(await screen.findByText("the operator paused the queue")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Resume All/ })).toBeTruthy();
  });

  it("explains a queue that is not available instead of failing silently", async () => {
    await openQueue({ queue: "unavailable" });
    expect(await screen.findByText("Post-processing is not available")).toBeTruthy();
  });

  it("imports a session folder", async () => {
    const calls: string[] = [];
    await openQueue({ queue: "none", calls });
    fireEvent.click(await screen.findByRole("button", { name: /Import Session/ }));
    const folder = screen.getByLabelText("Session folder");
    fireEvent.change(folder, { target: { value: "E:\\copy\\hall__talk" } });
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(calls).toContain("import_session"));
    expect(await screen.findByText("hall__talk is queued for transcription.")).toBeTruthy();
  });

  it("changes the model for the units not yet transcribed", async () => {
    const calls: string[] = [];
    await openQueue({ queue: "busy", calls });
    const card = await screen.findByLabelText("Job Parallel Track Sessions I");
    fireEvent.click(within(card).getByRole("button", { name: /Change Model/ }));
    expect(await screen.findByText("Applies to the units not yet transcribed")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(calls).toContain("queue_change_model"));
  });
});

describe("Record screen in record mode", () => {
  async function openRecord(opts: MockOptions) {
    mockIPC(bridge({ profile: true, capability: "tier-c-low", state: "recording", ...opts }));
    render(<App />);
    await screen.findByRole("heading", { name: "Parallel Track Sessions I" });
  }

  it("shows each committed segment with its language Badge and the parts as transcribed, transcribing and waiting", async () => {
    await openRecord({ queue: "busy" });
    expect(await screen.findByText(/Good afternoon, and welcome/)).toBeTruthy();
    expect(screen.getAllByText("HI·EN").length).toBe(2);
    expect(screen.getAllByText("UR").length).toBe(1);
    const parts = screen.getByLabelText("Parts");
    expect(within(parts).getAllByText("Transcribed").length).toBe(2);
    expect(within(parts).getByText("Transcribing 50%")).toBeTruthy();
    expect(within(parts).getAllByText("Waiting").length).toBe(1);
    expect(screen.getByText("Behind the room")).toBeTruthy();
    expect(screen.getByText("20 min")).toBeTruthy();
  });

  it("renders Urdu right to left and mixed rows with automatic direction", async () => {
    await openRecord({ queue: "busy" });
    const urdu = await screen.findByText(/اس مطالعے/);
    expect(urdu.getAttribute("dir")).toBe("rtl");
    const mixed = screen.getAllByText(/main finding/)[0] as HTMLElement;
    expect(mixed.getAttribute("dir")).toBe("auto");
    expect(mixed.getAttribute("lang")).toBe("hi");
  });

  it("locks a language with Alt+2 at the recorded time and returns to detection with Alt+4", async () => {
    const calls: string[] = [];
    await openRecord({ queue: "busy", calls });
    await screen.findByText(/Good afternoon, and welcome/);
    fireEvent.keyDown(window, { key: "2", altKey: true });
    await waitFor(() => expect(calls).toContain("set_language_lock"));
    expect(await screen.findByText(/Locked to Hindi/)).toBeTruthy();
    fireEvent.keyDown(window, { key: "4", altKey: true });
    await waitFor(() => expect(screen.getByText(/Per sentence/)).toBeTruthy());
  });

  it("shows the queue in the status bar", async () => {
    await openRecord({ queue: "busy" });
    expect(await screen.findByText(/Queue: 2 jobs · 1 running/)).toBeTruthy();
  });
});
