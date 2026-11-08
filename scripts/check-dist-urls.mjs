// Fails when the built UI (dist/) contains an http:// or https:// URL other than local IPC or
// localhost (PLAN.md section 10, Phase 0 Verification). Run after `pnpm build`.
//
// Allowed, because a browser never fetches them:
// - XML namespace identifiers (SVG, XLink, XHTML, MathML, XML), which React and every SVG file need
//   as plain strings.
// - React's minified-error text "https://react.dev/errors/<code>", which only appears inside a
//   thrown error message.
//
// Usage: node scripts/check-dist-urls.mjs [dir]   (default: dist)
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.argv[2] ?? "dist";
if (!existsSync(root)) {
  console.error(`check-dist-urls: ${root}/ not found; run pnpm build first`);
  process.exit(1);
}

const ALLOWED_EXACT = new Set([
  "http://www.w3.org/2000/svg",
  "http://www.w3.org/1999/xlink",
  "http://www.w3.org/1999/xhtml",
  "http://www.w3.org/1998/Math/MathML",
  "http://www.w3.org/XML/1998/namespace",
  "http://www.w3.org/2000/xmlns/",
]);
const ALLOWED_PREFIX = ["https://react.dev/errors/", "ipc://localhost"];
// Local hosts must match exactly: "http://localhost.evil.example" or "http://127.0.0.1.nip.io" fail.
const LOCAL_HOST =
  /^(?:https?|wss?):\/\/(?:localhost|127\.0\.0\.1|ipc\.localhost|tauri\.localhost)(?=[:/]|$)/;
const URL_PATTERN = /(?:https?|wss?|ftp):\/\/[^\s"'`<>()\\]*/g;
// Protocol-relative references (//host.tld/...) in CSS url() and HTML src/href.
const PROTOCOL_RELATIVE = /(?:url\(\s*["']?|(?:src|href)\s*=\s*["'])(\/\/[^\s"'`<>()\\]+)/g;
const TEXT = /\.(html|js|mjs|css|svg|json|txt|map|webmanifest)$/;

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* walk(path);
    else yield path;
  }
}

const problems = [];
let scanned = 0;
for (const file of walk(root)) {
  // Binary files (fonts, images) are scanned as latin1 so a URL inside metadata still shows.
  const text = readFileSync(file, TEXT.test(file) ? "utf8" : "latin1");
  scanned += 1;
  for (const match of text.matchAll(URL_PATTERN)) {
    const url = match[0];
    if (ALLOWED_EXACT.has(url)) continue;
    if (ALLOWED_PREFIX.some((prefix) => url.startsWith(prefix))) continue;
    if (LOCAL_HOST.test(url)) continue;
    problems.push(`${relative(process.cwd(), file)}: ${url}`);
  }
  for (const match of text.matchAll(PROTOCOL_RELATIVE)) {
    problems.push(`${relative(process.cwd(), file)}: ${match[1]}`);
  }
}

if (problems.length) {
  console.error([...new Set(problems)].join("\n"));
  console.error(`\ncheck-dist-urls: remote URL(s) found in ${root}/ (${scanned} files scanned)`);
  process.exit(1);
}
console.log(`check-dist-urls: ${scanned} files in ${root}/, no remote URLs`);
