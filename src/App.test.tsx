import { describe, expect, it } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { mockIPC } from "@tauri-apps/api/mocks";
import { App } from "./App";
import { initialRoute } from "./routes";
import { bridge } from "./test/mockBridge";
import type { MockState } from "./test/mockBridge";

function mockProfile(result: boolean | Error, state: MockState = "idle", calls?: string[]) {
  mockIPC(bridge({ profile: result, state, calls }));
}

describe("first launch", () => {
  it("opens System with a disabled Run System Check button when there is no capability profile", async () => {
    mockProfile(false);
    render(<App />);
    expect(await screen.findByText("Check This Computer Before the First Recording")).toBeTruthy();
    const run = screen.getByRole("button", { name: /Run System Check/ }) as HTMLButtonElement;
    expect(run.disabled).toBe(true);
    const system = screen.getByRole("button", { name: "System" });
    expect(system.getAttribute("aria-current")).toBe("page");
  });

  it("opens Sessions when a capability profile exists", async () => {
    mockProfile(true);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Sessions" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Sessions" }).getAttribute("aria-current")).toBe("page");
  });

  it("falls back to System and shows a Toast when the check fails", async () => {
    mockProfile(new Error("app data folder missing"));
    render(<App />);
    expect(await screen.findByText("Could not look for the system check profile")).toBeTruthy();
    expect(screen.getByText("Check This Computer Before the First Recording")).toBeTruthy();
  });

  it("initialRoute follows the profile", () => {
    expect(initialRoute(false)).toBe("system");
    expect(initialRoute(true)).toBe("sessions");
  });
});

describe("navigation", () => {
  it("has the six screens in website order and switches between them", async () => {
    mockProfile(true);
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    const nav = screen.getByRole("navigation", { name: "Main" });
    const labels = [...nav.querySelectorAll("button")].map((b) => b.textContent);
    expect(labels).toEqual(["Sessions", "Record", "Queue", "Review", "System", "Settings"]);
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(screen.getByRole("heading", { name: "No Session Open" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Queue" }));
    expect(screen.getByRole("heading", { name: "Queue" })).toBeTruthy();
  });

  it("gives the disabled live option a reason that follows the profile", async () => {
    mockProfile(false);
    const first = render(<App />);
    await screen.findByText("Check This Computer Before the First Recording");
    expect(screen.getByText("Not offered until the system check has run.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(screen.getByText("Not offered until the system check has run.")).toBeTruthy();
    first.unmount();

    mockProfile(true);
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    expect(screen.getByText("System check saved")).toBeTruthy();
    const saved = /Live captions are not built yet/;
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(screen.getByText(saved)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "System" }));
    expect(screen.getByText(saved)).toBeTruthy();
  });

  it("renders Hindi in Devanagari and Urdu right to left on the Record screen", async () => {
    mockProfile(true);
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    const urdu = screen.getByText(/اس مطالعے/);
    expect(urdu.getAttribute("lang")).toBe("ur");
    expect(urdu.getAttribute("dir")).toBe("rtl");
    const hindi = screen.getByText(/इस अध्ययन/);
    expect(hindi.getAttribute("lang")).toBe("hi");
    const mixed = screen.getByText(/main finding/);
    expect(mixed.getAttribute("dir")).toBe("auto");
  });
});

describe("recording", () => {
  it("starts a session from the Record screen and shows the recording state", async () => {
    const calls: string[] = [];
    mockProfile(true, "idle", calls);
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    const start = await screen.findByRole("button", { name: "Start Session" });
    await waitFor(() => expect((start as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(start);
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Session title/), { target: { value: "Keynote" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Start Recording" }));
    expect(await screen.findByRole("heading", { name: "Keynote" })).toBeTruthy();
    expect(calls).toContain("start_session");
    expect(screen.getByRole("button", { name: "Stop Session" })).toBeTruthy();
    expect(screen.getByText("Rec")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop Session" }));
    expect(await screen.findByText("Session stopped")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "No Session Open" })).toBeTruthy();
  });

  it("opens on Record with clock, parts, meter and backup while a session records", async () => {
    mockProfile(true, "recording");
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Parallel Track Sessions I" })).toBeTruthy();
    expect(screen.getByLabelText("Recorded time").textContent).toBe("00:45:15");
    const parts = screen.getByLabelText("Parts");
    expect(within(parts).getAllByText("Waiting")).toHaveLength(4);
    expect(within(parts).getByText("Recording")).toBeTruthy();
    expect(within(parts).getByText("40:00–")).toBeTruthy();
    expect(screen.getByText("part-0005.wav")).toBeTruthy();
    expect(screen.getByText("In sync")).toBeTruthy();
    expect(screen.getByRole("meter", { name: "Input level" })).toBeTruthy();
    expect(screen.getByText("Queue: no jobs · disk 388 GB free")).toBeTruthy();
  });

  it("shows the input-lost banner", async () => {
    mockProfile(true, "input_lost");
    render(<App />);
    expect((await screen.findByRole("alert")).textContent).toMatch(/Input lost/);
    expect(screen.getAllByText("Input lost").length).toBeGreaterThan(0);
  });

  it("offers Resume and Close for a session left open by a crash", async () => {
    const calls: string[] = [];
    mockProfile(true, "recovery", calls);
    render(<App />);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Resume This Session?")).toBeTruthy();
    expect(within(dialog).getByText(/1 part header repaired/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Resume Recording" }));
    await waitFor(() => expect(calls).toContain("recover_session"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("saves the backup folder from Settings", async () => {
    const calls: string[] = [];
    mockProfile(true, "idle", calls);
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const backup = (await screen.findByLabelText(/Backup folder/)) as HTMLInputElement;
    await waitFor(() => expect(backup.value).toBe(String.raw`E:\STELMAR-Backup`));
    fireEvent.change(backup, { target: { value: String.raw`F:\Backup` } });
    fireEvent.click(screen.getByRole("button", { name: "Save Folders" }));
    expect(await screen.findByText("Folders saved")).toBeTruthy();
    expect(calls).toContain("save_settings");
  });
});
