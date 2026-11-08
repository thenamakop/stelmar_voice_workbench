// Fails when src/ contains a raw hex colour or a raw px length (AGENTS.md non-negotiable 5, PLAN.md
// section 9). ESLint's adherence rules only see JS string literals; this also covers CSS files,
// template literals and JSX text. Use design tokens through var(), or plain numbers in React style
// objects as the design-system components do.
//
// Usage: node scripts/check-style-values.mjs [dir]   (default: src)
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.argv[2] ?? "src";
const EXTENSIONS = /\.(css|ts|tsx|js|jsx|mjs|html)$/;
const CHECKS = [
  { name: "raw hex colour", pattern: /#[0-9a-fA-F]{3,8}\b/g },
  { name: "raw px value", pattern: /\b\d+(\.\d+)?px\b/g },
  { name: "raw rgb()/hsl() colour", pattern: /\b(?:rgb|hsl)a?\(/g },
];

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* walk(path);
    // Tests assert computed styles ("32px"), so they are skipped.
    else if (EXTENSIONS.test(name) && !/\.test\.[jt]sx?$/.test(name)) yield path;
  }
}

const problems = [];
for (const file of walk(root)) {
  const lines = readFileSync(file, "utf8").split(/\r?\n/);
  lines.forEach((line, i) => {
    for (const { name, pattern } of CHECKS) {
      for (const match of line.matchAll(pattern)) {
        problems.push(`${relative(process.cwd(), file)}:${i + 1}: ${name} '${match[0]}'`);
      }
    }
  });
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} raw style value(s) in ${root}/. Use design tokens.`);
  process.exit(1);
}
console.log(`check-style-values: no raw hex or px values in ${root}/`);
