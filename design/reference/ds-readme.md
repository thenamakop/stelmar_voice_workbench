# STELMAR Design System

STELMAR is the **Scholars' Conference on Sustainable Trends in Engineering, Law, Management, and Liberal Arts Research**, hosted by **BML Munjal University** (a Hero Group initiative). The 2nd edition, STELMAR 2026, runs **29–31 January 2026** with the theme *"Sustainable Systems and Interdisciplinary Innovations for a Resilient Future."* It's a peer-reviewed conference for publishing and presenting papers.

Surfaces:
1. **Conference website** — public site. Nav: Guidelines, Call for Papers, Speakers, Registrations, Sponsorship, Committee, Contact, Timeline, Portal.
2. **Author portal** — the "Portal" link: authors submit and track papers.

## Sources
- The brief: "STELMAR: a reputed conference for publishing and presenting conference papers."
- `uploads/pasted-1793198649696-0.png` — a screenshot of the live site, given as a **theme reference** (colour, type, mood), **not a layout to copy**. Colours were sampled from it; the logo lockup was cropped from it (`assets/`).
- No codebase, Figma, font files or vector logos were provided. All screens are flexible demonstrations of the theme; content lives in data, not in components.

---

## Content fundamentals
- **Voice:** formal and academic, but welcoming. Use full official names ("2nd Scholars' Conference on Sustainable Trends in Engineering, Law, Management, and Liberal Arts Research 2026") in heroes, and "STELMAR 2026" elsewhere.
- **Person:** the conference speaks as **we**, and addresses authors as **you**.
- **Casing:** Title Case for buttons and headings, as on the live site ("Submit Paper", "Call for Papers"). The nav is UPPERCASE. The theme line is Title Case.
- **Dates:** ordinals with superscripts, month spelled out: "29th, 30th & 31st January 2026". Short form: "29 Jan 2026". Times use 24-hour IST.
- **Buttons** are short and verb-led: "Submit Paper", "About", "Register".
- **Emoji:** never. The only glyph used is the line icon (calendar) inside the date pill.
- **Spelling:** Indian/British English (programme, organise).

## Visual foundations
- **Vibe:** a sober charcoal-and-white academic site. Colour comes only from the logo's rainbow book fan and from photography.
- **Colour:** the brand is **charcoal**. The header is #353A3E (`--ink-700`), primary buttons are #222526 (`--ink-900`), and the light button is #D9DADD (`--ink-200`). Pages are white. The **logo spectrum** (orange #F06018, amber, yellow #FFC060, green #78A860, teal #48C0A8, sky, blue #6090C0) is for small accents only: the active nav underline, status, and track markers. Never use it for large fills. Use the darker `-700` steps for text on white.
- **Type:** **Inter** throughout. Hero is 700, 60/76, white, centred, and balanced. Headings are 700. Nav is 500 at 17px, uppercase. Body is 400 at 16/26. IBM Plex Mono is a system addition, used only for paper IDs and times.
- **Hero pattern (optional, composable):** photography blurred under a ~55% charcoal overlay with white bold type. Compose freely from `Hero`, `GlassPill`, `Button` and `Countdown` — centred or left-aligned, with any subset of elements. Not every page needs a photo hero; solid charcoal or white sections are equally on-brand.
- **Glass / transparency:** used **only on photography**. White at 16% (26% for digit tiles), a 38% white border, and a 10px backdrop blur, with pill radius (999px) for chips and 6–10px for the countdown.
- **Backgrounds:** solid charcoal (header, footer) or white. No gradients or textures. Imagery is campus and documentary photography, desaturated by blur and overlay.
- **Corner radii:** buttons 4px, cards 6px, dialogs 8px, glass chips and tags as pills.
- **Cards:** white, 1px #D9DADD hairline, 6px radius, no shadow at rest. Hover darkens the border and adds shadow-1.
- **Shadows:** minimal. Shadow-2 for toasts and popovers, shadow-3 for dialogs. No inner shadows.
- **Hover / press:** one step darker (#222526→#353A3E, #D9DADD→#C3C6C9). Glass gets more opaque. No scaling.
- **Focus:** a blue-600 border plus a soft blue-100 ring.
- **Motion:** 120/200/320ms with ease cubic-bezier(.2,0,0,1). Fades only, no bounce. The countdown ticks every second.
- **Layout:** no fixed page template. Common: a charcoal `SiteHeader` (logo left, uppercase nav right), white content in ≤1200px containers, and a charcoal footer.

## Iconography
- The site's calendar glyph matches **Lucide** (outline, 2px stroke), so Lucide is used via CDN (`lucide-static@0.460.0`) through the `Icon` component, inheriting `currentColor`.
- No emoji, no icon fonts.
- **Logo:** `assets/logo-lockup-header.png` (STELMAR + BMU), `assets/stelmar-logo-header.png`, and `assets/bmu-logo-header.png`. These are low-res crops from the screenshot with the charcoal background baked in, so use them on #353A3E only. Vector or transparent masters are needed.

## Fonts
Inter is loaded from Google Fonts and matches the live site visually. Supply self-hosted files if the site uses a specific build.

---

## Index
- `styles.css` — entry point (imports only)
- `tokens/` — `fonts.css`, `colors.css`, `typography.css`, `spacing.css`, `effects.css`, `base.css`
- `guidelines/` — foundation specimen cards (Colors, Type, Spacing, Brand)
- `components/` — React primitives (below), one card per folder
- `assets/` — logo crops, hero reference
- `ui_kits/website/` — conference website (theme demo; content in content.js)
- `ui_kits/author-portal/` — author portal (Sign in, My submissions, New submission)
- `thumbnail.html`, `SKILL.md`

## Components
- **core/** — Button, IconButton, Icon, Badge, Tag
- **forms/** — Input (incl. multiline), Select, Checkbox, Radio, Switch
- **layout/** — Card, PaperCard, SessionItem
- **navigation/** — Tabs
- **site/** — SiteHeader, Hero, GlassPill, Countdown
- **feedback/** — Dialog, Toast, Tooltip

### Intentional additions
- **Icon** — wrapper for the Lucide CDN glyph set.
- Button `light` and `glass` variants, plus **site/** components (SiteHeader, Hero, GlassPill, Countdown), are theme building blocks taken from the reference site.
- **PaperCard** — the core object of a conference: one submission/paper with ID, track, status, authors.
- **SessionItem** — one programme row; repeated across the site's schedule.
