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
  it("opens System with a Run Quick Check button when there is no capability profile", async () => {
    mockProfile(false);
    render(<App />);
    expect(await screen.findByText("Check This Computer Before the First Recording")).toBeTruthy();
    const run = screen.getByRole("button", { name: /Run Quick Check/ }) as HTMLButtonElement;
    expect(run.disabled).toBe(false);
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
    expect(screen.getByText(/Fixture · UHD/)).toBeTruthy();
    const saved = /Live captions are chosen in the Start Session dialog/;
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(screen.getByText(saved)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "System" }));
    expect(screen.getByText(/Live captions are not offered on this computer/)).toBeTruthy();
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

function mockCapability(name: string, extra: { fingerprintChange?: string[]; calls?: string[] } = {}) {
  mockIPC(bridge({ capability: name, calls: extra.calls, fingerprintChange: extra.fingerprintChange }));
}

describe("the System screen with a saved profile", () => {
  it("shows the tier, the hardware, the speed test and the verdict", async () => {
    mockCapability("tier-c-low");
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "System" }));
    expect(await screen.findByRole("heading", { name: "Record Now, Transcribe in the Background" })).toBeTruthy();
    expect(screen.getByText("Tier C")).toBeTruthy();
    expect(screen.getByText("Processor")).toBeTruthy();
    expect(screen.getAllByText("Chosen").length).toBeGreaterThan(0);
    expect(screen.getByText("Ready after a 90-minute talk")).toBeTruthy();
    expect(screen.getByText("Fingerprint 3F2A-91C7")).toBeTruthy();
  });

  it("asks before it turns a safeguard off and logs the change", async () => {
    const calls: string[] = [];
    mockCapability("tier-c-low", { calls });
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "System" }));
    const sw = await screen.findByRole("switch", { name: "Pause on battery power" });
    fireEvent.click(sw);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/runs on its battery/)).toBeTruthy();
    expect(calls).not.toContain("set_override");
    fireEvent.click(within(dialog).getByRole("button", { name: "Turn off" }));
    await waitFor(() => expect(calls).toContain("set_override"));
    await waitFor(() => expect((screen.getByRole("switch", { name: "Pause on battery power" }) as HTMLInputElement).checked).toBe(false));
  });

  it("shows the banner when the profile no longer matches the machine", async () => {
    mockCapability("tier-c-low", { fingerprintChange: ["the graphics driver changed"] });
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "System" }));
    expect(await screen.findByText(/has changed since the last check/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Re-run Quick Check" })).toBeTruthy();
  });

  it("calls a machine that only records by that name", async () => {
    mockCapability("tier-r");
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "System" }));
    expect(await screen.findByRole("heading", { name: "This Computer Records Only" })).toBeTruthy();
  });
});

describe("the Start Session dialog", () => {
  async function openDialog(name: string) {
    mockCapability(name);
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    const start = await screen.findByRole("button", { name: "Start Session" });
    await waitFor(() => expect((start as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(start);
    return await screen.findByRole("dialog");
  }

  it("defaults to Record + post-process, English + Hindi and Devanagari, and says why live is off", async () => {
    const dialog = await openDialog("tier-c-low");
    expect((within(dialog).getByRole("radio", { name: /Record \+ post-process/ }) as HTMLInputElement).checked).toBe(true);
    const live = within(dialog).getByRole("radio", { name: /Live captions \+ post-process/ }) as HTMLInputElement;
    expect(live.disabled).toBe(true);
    expect((within(dialog).getByRole("checkbox", { name: "English" }) as HTMLInputElement).checked).toBe(true);
    expect((within(dialog).getByRole("checkbox", { name: "Hindi / Urdu" }) as HTMLInputElement).checked).toBe(true);
    expect((within(dialog).getByRole("radio", { name: /Devanagari/ }) as HTMLInputElement).checked).toBe(true);
    await waitFor(() => expect(within(dialog).getByText(/runs slower than real time/)).toBeTruthy());
  });

  it("offers live only on a machine the check cleared", async () => {
    const dialog = await openDialog("tier-g");
    await waitFor(() =>
      expect((within(dialog).getByRole("radio", { name: /Live captions \+ post-process/ }) as HTMLInputElement).disabled).toBe(false),
    );
    expect((within(dialog).getByRole("radio", { name: /Record \+ post-process/ }) as HTMLInputElement).checked).toBe(true);
  });

  it("sends the chosen languages and the script, in Record + post-process mode", async () => {
    const calls: string[] = [];
    mockIPC(bridge({ capability: "tier-c-low", calls }));
    render(<App />);
    await screen.findByRole("heading", { name: "Sessions" });
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    const start = await screen.findByRole("button", { name: "Start Session" });
    await waitFor(() => expect((start as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(start);
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "English" }));
    fireEvent.click(within(dialog).getByRole("radio", { name: /Nastaliq/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Start Recording" }));
    await waitFor(() => expect(calls).toContain("start_session"));
  });
});
