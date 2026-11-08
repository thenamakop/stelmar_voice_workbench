// @vitest-environment node
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import aliasFile from "../../scripts/icon-aliases.json";

// Every icon name the UI uses must exist in lucide-static@0.460.0 (or be an alias copied by
// scripts/copy-icons.mjs); a missing mask image renders as nothing.
const require = createRequire(join(process.cwd(), "package.json"));
const iconDir = join(dirname(require.resolve("lucide-static/package.json")), "icons");
const aliases: Record<string, string> = aliasFile.aliases;

// Names chosen inside ported components rather than passed as JSX attributes.
const INTERNAL = [
  "check-circle",
  "info",
  "alert-triangle",
  "alert-circle",
  "x",
  "check",
  "chevron-down",
  "bookmark",
  "bookmark-check",
];

function* sources(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* sources(path);
    else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) yield path;
  }
}

function usedIcons(): string[] {
  const names = new Set(INTERNAL);
  const attr = /\b(?:name|icon|iconLeft|iconRight)=["']([a-z0-9-]+)["']/g;
  for (const file of sources(join(process.cwd(), "src"))) {
    for (const m of readFileSync(file, "utf8").matchAll(attr)) if (m[1]) names.add(m[1]);
  }
  return [...names].sort();
}

describe("icons", () => {
  it("every icon the UI uses exists locally", () => {
    const missing = usedIcons().filter((name) => {
      const real = aliases[name] ?? name;
      return !existsSync(join(iconDir, `${real}.svg`));
    });
    expect(missing).toEqual([]);
  });

  it("finds icons used by the screens", () => {
    expect(usedIcons()).toEqual(expect.arrayContaining(["refresh-cw", "pause", "plus", "search"]));
  });
});
