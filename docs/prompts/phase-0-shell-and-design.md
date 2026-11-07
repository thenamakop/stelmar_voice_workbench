# Phase 0: App shell in the STELMAR design

## Context
This repository holds the STELMAR Voice Workbench kit: `AGENTS.md`, `docs/PLAN.md`, the frozen STELMAR design system in `design/`, and reference mockups in `design/mockups/`. We are building an offline Tauri 2 desktop app that records conference sessions and transcribes them in English, Hindi and Urdu on the same machine, with live captions only where the hardware allows. Nothing exists yet. This phase builds the skeleton and the look; no audio or speech yet.

## Goal
A Tauri 2.11 + React + TypeScript app that opens fully offline into the STELMAR-styled shell (top bar, six screens, status bar), built only from the design system's tokens, local fonts and ported components; a Cargo workspace ready for the engine crates; lint and CI on Windows.

## Steps
1. Scaffold with `pnpm create tauri-app` (React, TypeScript) at the repository root, keeping the kit folders. Make a Cargo workspace with `src-tauri`, `crates/wb-protocol` (library, empty serde types for now) and `crates/wb-engine` (binary that prints its version). Pin tauri 2.11.x crates and `@tauri-apps/cli` 2.11.x. Product name "STELMAR Voice Workbench", identifier `in.edu.bmu.stelmar.voiceworkbench`.
2. Styles: import `design/tokens/{colors,typography,spacing,effects,base}.css` unchanged, then `design/tokens/app-additions.css` and `design/fonts.css`. Bundle the font files through Vite; no Google Fonts or other remote URLs.
3. Port these components from `design/reference/ds-bundle.js` into `src/ds/` with identical props, defaults and inline style objects, typed in TypeScript: Button, IconButton, Icon, Badge, Tag, Input, Select, Checkbox, Switch, Card, Tabs, Dialog, Toast, Tooltip, SessionItem. Icon keeps the CSS-mask technique but loads `/icons/<name>.svg`; a prebuild script copies the SVGs from `lucide-static@0.460.0`. Add one app component, `ProgressBar`, built from tokens as `.prog` in `design/mockups/mockup.css`.
4. Shell per `design/mockups/record-only.html`: charcoal top bar with `design/brand/logo-lockup-header.png`, the "Voice Workbench" label and nav Sessions, Record, Queue, Review, System, Settings (website nav style, orange underline on the active item); a status bar; one route per screen with placeholder content made of real components. On first launch (no `capability.json` in the app data folder) the app opens System with a disabled "Run System Check" button, wired in Phase 3.
5. Offline from day one: Content Security Policy in `tauri.conf.json` (`app.security.csp`) allowing only `'self'`, local assets, fonts, images and the IPC origin; capabilities granting only core window events; no plugins.
6. Lint and format: TypeScript strict; oxlint (or ESLint) with `design/reference/adherence.oxlintrc.json`, extended only to allow the font families "Noto Sans Devanagari", "Noto Nastaliq Urdu" and "Noto Naskh Arabic"; rustfmt and `cargo clippy --workspace --all-targets -- -D warnings`.
7. Web-UI test harness: Vitest plus Playwright running the Vite build in Chromium with `mockIPC` from `@tauri-apps/api/mocks`; tests screenshot the Record and System placeholders at 1536x864.
8. CI (GitHub Actions, `windows-latest`): pnpm install, lint, typecheck, Vitest, Playwright, `cargo clippy` and `cargo test` for the workspace, and `pnpm tauri build`. Set `GGML_NATIVE=OFF` in the CI environment from now on.

## Verification
- `pnpm tauri dev` opens the shell. With Wi-Fi and Ethernet disabled, Inter, IBM Plex Mono, Devanagari and Urdu sample text all render (screenshot attached).
- Screenshots of Record and System sit next to `design/mockups/record-only.png` and `system-check.png` in the PR; bar, nav, overline, cards and status bar match.
- `pnpm lint` passes with the adherence config; no raw hex or px values in `src/`.
- No `http://` or `https://` strings in `dist/` other than IPC or localhost (scripted check in CI).
- CI is green. `design-consistency-reviewer` and `offline-auditor` report PASS.

## If Blocked
- A component can't be ported one-to-one to React 19: keep the visual output identical and note the difference in the PR.
- WebView2 is missing on a test machine: install the Evergreen runtime for development; the offline installer mode comes in Phase 8.
