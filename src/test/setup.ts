import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";

afterEach(() => {
  // Tests that run in the node environment (no DOM) have nothing to clean up.
  if (typeof window === "undefined") return;
  cleanup();
  clearMocks();
});
