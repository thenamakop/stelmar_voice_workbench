# STELMAR design files for the Voice Workbench

The app must look like the STELMAR website. These files are the source of truth.

| Path | Status | Notes |
| --- | --- | --- |
| `tokens/colors.css`, `typography.css`, `spacing.css`, `effects.css`, `base.css` | Frozen | Byte-identical copies from the STELMAR design system. Never edit. |
| `tokens/app-additions.css` | App-owned | Devanagari and Urdu font stacks and line heights, caption sizes, shell sizes. Add here only, with a reason. |
| `fonts.css`, `fonts/` | App-owned | Offline Inter, IBM Plex Mono, Noto Sans Devanagari, Noto Nastaliq Urdu, Noto Naskh Arabic (all SIL OFL 1.1, licences in `fonts/LICENSES/`). Replaces the website's Google Fonts import. |
| `brand/` | Frozen | Logo lockup and marks from the website. The lockup has the charcoal `#353A3E` background baked in: use it only on `--surface-header`. |
| `reference/ds-bundle.js` | Reference | The design system's components (readable Babel output). Port components from here with identical props and styles. |
| `reference/ds-manifest.json`, `ds-readme.md` | Reference | Component list and the design rules (voice, colour, type, radii, motion). |
| `reference/adherence.oxlintrc.json` | Reference | The design system's lint rules (no raw hex or px, valid component props). Extend only to allow the three Noto families. |
| `reference/website-site.css` | Reference | The website's own CSS: header, nav underline, overline with the orange dot, cards (`.box`), tables (`.tbl`). |
| `mockups/start-session.*`, `record-only.*`, `record-live.*`, `system-check.*`, `captions.*` | Reference | Layout targets for the Start Session dialog, the Record screen in both modes and the System check (1536x864) and the projector caption window (1920x1080), built only from these tokens and fonts. Numbers in them are placeholders. `mockup.css` mirrors the design-system components, plus the Switch, progress bar and app-density table used by the new screens. |

Rules in short: charcoal and white; the logo spectrum only as small accents; Inter for UI; IBM Plex Mono only for times, IDs, file and model names; Title Case buttons; no emoji; buttons 4px radius, cards 6px, dialogs 8px; 120, 200 and 320 ms fades.
