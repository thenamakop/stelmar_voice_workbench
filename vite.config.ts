import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import process from "node:process";
import { fileURLToPath } from "node:url";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(({ mode }) => {
  // `vite build --mode e2e` builds the Playwright bundle (mocked Tauri bridge) into dist-e2e/,
  // so the shipped dist/ never contains test code.
  const e2e = mode === "e2e";
  return {
    plugins: [react()],
    // Keep Rust errors visible in `tauri dev`.
    clearScreen: false,
    build: {
      // Never inline assets as data: URIs; fonts, icons and images ship as files so the
      // Content Security Policy can stay at 'self'.
      assetsInlineLimit: 0,
      // WebView2 supports modulepreload natively; drop Vite's fetch()-based polyfill.
      modulePreload: { polyfill: false },
      target: "es2022",
      outDir: e2e ? "dist-e2e" : "dist",
      rollupOptions: e2e ? { input: fileURLToPath(new URL("./e2e/index.html", import.meta.url)) } : undefined,
    },
    server: {
      // Tauri expects a fixed port.
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
      watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
    },
  };
});
