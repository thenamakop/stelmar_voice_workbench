import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { mockIPC } from "@tauri-apps/api/mocks";
import { App } from "../App";
import { REGISTRY, bridge } from "../test/mockBridge";
import { EnginesSection, ModelsSection, formatBytes } from "./RegistrySections";
import type { RegistrySnapshot } from "../ipc";

const registry = REGISTRY as unknown as RegistrySnapshot;

describe("formatBytes", () => {
  it("uses decimal units", () => {
    expect(formatBytes(1_533_763_059)).toBe("1.5 GB");
    expect(formatBytes(487_601_967)).toBe("488 MB");
    expect(formatBytes(2_000)).toBe("2 kB");
  });
});

describe("Speech engines section", () => {
  it("shows each build, its devices and its result", () => {
    render(<EnginesSection registry={registry} />);
    expect(screen.getByText("avx2")).toBeTruthy();
    expect(screen.getAllByText("Ready").length).toBe(2);
    expect(screen.getByText("Needs a newer CPU")).toBeTruthy();
    expect(screen.getByText(/needs CPU features this computer does not have/)).toBeTruthy();
    // The integrated GPU with its driver and the RAM it shares.
    expect(
      screen.getByText(
        /Iris\(R\) Xe Graphics.*integrated graphics.*shares 8\.6 GB of RAM.*driver 31\.0\.101\.4577/,
      ),
    ).toBeTruthy();
  });

  it("says plainly that recording works when no engine is found", () => {
    render(<EnginesSection registry={{ ...registry, engines: [], engines_done: true }} />);
    expect(screen.getByText("None found")).toBeTruthy();
    expect(screen.getByText("No engine")).toBeTruthy();
    expect(screen.getByText(/Recording works without an engine/)).toBeTruthy();
  });

  it("shows Checking while the probe runs", () => {
    render(<EnginesSection registry={null} />);
    expect(screen.getByText("Looking…")).toBeTruthy();
  });
});

describe("Models section", () => {
  it("marks verified, altered and missing files", () => {
    render(<ModelsSection registry={registry} />);
    expect(screen.getAllByText("Verified").length).toBe(2);
    expect(screen.getByText("Checksum differs")).toBeTruthy();
    expect(screen.getByText("Missing")).toBeTruthy();
    expect(screen.getAllByText("EN, HI, UR", { selector: "td" }).length).toBe(2);
    expect(screen.getByText(/A model whose checksum does not match is disabled and never used/)).toBeTruthy();
  });

  it("shows the manifest problem when there is one", () => {
    render(
      <ModelsSection
        registry={{
          ...registry,
          models: [],
          manifest_error: "manifest.json is not valid",
          models_done: true,
        }}
      />,
    );
    expect(screen.getByText("Not installed")).toBeTruthy();
    expect(screen.getByText("manifest.json is not valid")).toBeTruthy();
  });
});

describe("System screen", () => {
  it("loads the registry from the app", async () => {
    mockIPC(bridge({ profile: false }));
    render(<App />);
    expect(await screen.findByText("Speech engines")).toBeTruthy();
    expect(await screen.findByText("wb-engine-avx2.exe · 5 MB")).toBeTruthy();
  });
});
