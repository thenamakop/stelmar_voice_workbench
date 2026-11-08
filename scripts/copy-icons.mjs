// Copies the Lucide icon SVGs from lucide-static@0.460.0 into public/icons/ so the Icon component
// can load /icons/<name>.svg from the app itself. The design system loads them from unpkg; the app
// is offline, so they ship as local files (AGENTS.md non-negotiable 1). public/icons/ is git-ignored.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const EXPECTED_VERSION = "0.460.0";
const require = createRequire(import.meta.url);
const pkgJson = require.resolve("lucide-static/package.json");
const { version } = JSON.parse(readFileSync(pkgJson, "utf8"));
if (version !== EXPECTED_VERSION) {
  console.error(`copy-icons: lucide-static is ${version}, expected ${EXPECTED_VERSION} (AGENTS.md)`);
  process.exit(1);
}

const pkgDir = dirname(pkgJson);
const source = join(pkgDir, "icons");
const target = join("public", "icons");
if (existsSync(target)) rmSync(target, { recursive: true, force: true });
mkdirSync(target, { recursive: true });
cpSync(source, target, { recursive: true });
// The design-system bundle still uses a few pre-rename Lucide names (Toast); copy them under the old
// names so the ports stay identical to ds-bundle.js.
const { aliases } = JSON.parse(readFileSync(new URL("./icon-aliases.json", import.meta.url), "utf8"));
for (const [alias, real] of Object.entries(aliases)) {
  const from = join(target, `${real}.svg`);
  if (!existsSync(from)) {
    console.error(`copy-icons: alias ${alias} points at missing icon ${real}`);
    process.exit(1);
  }
  cpSync(from, join(target, `${alias}.svg`));
}

const licence = join(pkgDir, "LICENSE");
if (existsSync(licence)) cpSync(licence, join(target, "LICENSE.txt"));

const count = readdirSync(target).filter((f) => f.endsWith(".svg")).length;
console.log(`copy-icons: ${count} icons from lucide-static@${version} -> ${target}`);
