// Entry point for the Playwright build only (vite build --mode e2e, input e2e/index.html).
// It mocks the Tauri bridge with mockIPC before the app starts. Query string:
// ?profile=present (capability.json exists) or anything else (first launch).
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const profile = new URLSearchParams(window.location.search).get("profile");
mockWindows("main");
mockIPC((cmd) => {
  if (cmd === "has_capability_profile") return profile === "present";
  throw new Error(`e2e: command ${cmd} is not mocked`);
});

await import("../main");
