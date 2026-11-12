// Entry point for the Playwright build only (vite build --mode e2e, input e2e/index.html).
// It mocks the Tauri bridge with mockIPC before the app starts. Query string:
// ?profile=present (capability.json exists) or anything else (first launch);
// &state=idle|recording|input_lost|recovery (the recorder's state, src/test/mockBridge.ts);
// &engines=none (every speech engine renamed away: list_engines finds none).
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { bridge } from "./mockBridge";
import type { MockState } from "./mockBridge";

const params = new URLSearchParams(window.location.search);
mockWindows("main");
mockIPC(
  bridge({
    profile: params.get("profile") === "present",
    state: (params.get("state") ?? "idle") as MockState,
    engines: params.get("engines") === "none" ? "none" : "present",
  }),
);

await import("../main");
