import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { mockIPC } from "@tauri-apps/api/mocks";
import { App } from "./App";
import { initialRoute } from "./routes";

function mockProfile(result: boolean | Error) {
  mockIPC((cmd) => {
    if (cmd !== "has_capability_profile") throw new Error(`unexpected command ${cmd}`);
    if (result instanceof Error) throw result;
    return result;
  });
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
