// ESLint flat config for the STELMAR Voice Workbench UI.
//
// The design-adherence rules come straight from design/reference/adherence.oxlintrc.json (the
// design system's own lint file, which stays untouched). They are read at lint time, so any update
// to the reference flows through. Two changes: the font-family rule also allows the three Noto
// families the app adds for Hindi and Urdu (PLAN.md section 9), and the component-prop rules also
// allow standard pass-through attributes (see PASS_THROUGH). The reference's warnings fail the lint
// because `pnpm lint` runs with --max-warnings 0.
//
// Why ESLint and not oxlint: oxlint cannot parse the reference file (it carries an `x-omelette`
// block) and does not implement `no-restricted-syntax`, which holds every adherence rule.
import { readFileSync } from "node:fs";
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

const reference = JSON.parse(
  readFileSync(new URL("./design/reference/adherence.oxlintrc.json", import.meta.url), "utf8"),
);

const DS_FONTS = "(?:Inter|IBM Plex Mono)";
const APP_FONTS = "(?:Inter|IBM Plex Mono|Noto Sans Devanagari|Noto Nastaliq Urdu|Noto Naskh Arabic)";

// The reference's "doesn't accept that prop" rules list only each component's design props, so they
// also flag event handlers, ARIA and standard form attributes (the design system's own screens trip
// them: <Button onClick>, <Input placeholder>). Under --max-warnings 0 that would forbid any
// interaction, so these pass-through attributes are allowed as well. The lists below, not TypeScript,
// limit the extra attributes (several ports extend the HTML attribute types); TypeScript rejects
// unknown design props and invalid values.
const PROP_RULE = "> JSXIdentifier[name!=/^(?:";
const PASS_THROUGH =
  "on[A-Z][A-Za-z]*|aria-[a-z-]+|data-[a-z-]+|id|name|title|type|value|defaultValue|" +
  "placeholder|disabled|readOnly|autoFocus|autoComplete|tabIndex|role|";

// App additions to the same rule (one rule key, so they must share its array). They close gaps the
// reference leaves in React style objects: font families and functional colours.
const APP_SELECTORS = [
  {
    selector: "Property[key.name='fontFamily'] > Literal[value!=/^var\\(--font-[a-z]+\\)$/]",
    message: "fontFamily must be a font token, for example var(--font-ui) or var(--font-devanagari).",
  },
  {
    selector: "Literal[value=/\\b(?:rgb|hsl)a?\\(/]",
    message: "Raw rgb()/hsl() colour — use a design-system color token via var().",
  },
];

// px() (src/ds/units.ts) builds pixel lengths from numbers. Ports use it to keep ds-bundle.js
// values; the shell and screens use it only for mockup.css values that have no token. Prefer a
// token whenever one equals the value.

function adherenceRules() {
  const rules = {};
  for (const [name, setting] of Object.entries(reference.rules)) {
    if (name === "react/forbid-elements") {
      // The reference forbids no elements (empty list), so this rule is a no-op; it is skipped to
      // avoid adding eslint-plugin-react for nothing. Fail loudly if the reference ever fills it.
      const [, options] = setting;
      if (options?.forbid?.length) {
        throw new Error("adherence: react/forbid-elements now has entries; add eslint-plugin-react");
      }
      continue;
    }
    if (name === "no-restricted-syntax") {
      const [level, ...selectors] = setting;
      let replaced = 0;
      let propRules = 0;
      const patched = selectors.map((entry) => {
        if (entry.selector.includes(PROP_RULE)) {
          propRules += 1;
          return { ...entry, selector: entry.selector.replace(PROP_RULE, PROP_RULE + PASS_THROUGH) };
        }
        if (!entry.selector.includes("font-family")) return entry;
        if (!entry.selector.includes(DS_FONTS)) {
          throw new Error("adherence: font-family selector changed shape; update eslint.config.js");
        }
        // The reference lookahead `(?!['"]?...)` follows `\s*`, which backtracks to zero spaces, so it
        // flags every font, Inter included. Letting the lookahead skip the spaces itself fixes that.
        const LOOKAHEAD = "(?!['\\\"]?";
        if (!entry.selector.includes(LOOKAHEAD)) {
          throw new Error("adherence: font-family lookahead changed shape; update eslint.config.js");
        }
        replaced += 1;
        return {
          selector: entry.selector.replace(DS_FONTS, APP_FONTS).replace(LOOKAHEAD, "(?![\\s'\\\"]*"),
          message:
            "Font not provided by the design system or the app. Available: Inter, IBM Plex Mono, " +
            "Noto Sans Devanagari, Noto Nastaliq Urdu, Noto Naskh Arabic.",
        };
      });
      if (replaced !== 1) throw new Error("adherence: expected exactly one font-family selector");
      if (propRules === 0) throw new Error("adherence: no component prop rules found; check the reference");
      rules[name] = [level, ...patched, ...APP_SELECTORS];
      continue;
    }
    rules[name] = setting;
  }
  return rules;
}

export default tseslint.config(
  {
    ignores: ["dist/**", "src-tauri/**", "target/**", "node_modules/**", "public/**", "design/**"],
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: "module",
      globals: globals.browser,
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      ...adherenceRules(),
    },
  },
  {
    // Tests assert the computed styles of the ports ("32px"), so raw values are allowed there.
    files: ["src/**/*.test.{ts,tsx}"],
    rules: { "no-restricted-syntax": "off" },
  },
);
