// Checks models/manifest.json: it holds metadata only (AGENTS.md: never commit model binaries or
// private paths) and no ignore pattern hides it.
//
//   node scripts/check-manifest.mjs
//
// Fails on: an unknown key, a local path (drive letter, backslash, "..", a leading slash), a token or
// credentialed URL, a malformed SHA-256, a duplicate id, a missing source revision, and a git ignore
// rule that matches models/manifest.json.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const FILE = join("models", "manifest.json");
const ENTRY_KEYS = new Set([
  "id", "file", "sha256", "bytes", "kind", "base", "quant", "languages", "candidate_roles",
  "validated_roles", "licence", "source", "conversion",
]);
const REQUIRED = ["id", "file", "sha256", "bytes", "licence", "source"];
const CONVERSION_KEYS = new Set(["script", "settings", "input_sha256", "whisper_cpp_commit", "deterministic"]);
const TOP_KEYS = new Set(["version", "_note", "models"]);

const problems = [];
const bad = (m) => problems.push(m);

if (!existsSync(FILE)) {
  console.error(`${FILE} does not exist`);
  process.exit(1);
}
const text = readFileSync(FILE, "utf8");
let doc;
try {
  doc = JSON.parse(text);
} catch (e) {
  console.error(`${FILE}: not valid JSON: ${e.message}`);
  process.exit(1);
}

for (const k of Object.keys(doc)) if (!TOP_KEYS.has(k)) bad(`top-level key '${k}' is not allowed`);
if (!Array.isArray(doc.models) || doc.models.length === 0) bad("'models' must be a non-empty array");

// Text that must never appear anywhere in the file.
const forbidden = [
  [/[A-Za-z]:[\\/]/, "a drive letter path"],
  [/\\/, "a backslash"],
  [/\.\.[\\/]/, "a '..' path"],
  [/hf_[A-Za-z0-9]{10,}/, "a Hugging Face token"],
  [/token/i, "the word 'token'"],
  [/https?:\/\/[^\s"/]*@/, "a credentialed URL"],
  [/\bAppData\b|\bUsers\b/, "a user profile path"],
];
for (const [re, what] of forbidden) if (re.test(text)) bad(`the file contains ${what}`);

const ids = new Set();
const sha = /^[0-9a-f]{64}$/;
for (const [i, m] of (doc.models ?? []).entries()) {
  const where = `models[${i}] (${m.id ?? "?"})`;
  for (const k of Object.keys(m)) if (!ENTRY_KEYS.has(k)) bad(`${where}: key '${k}' is not allowed`);
  for (const k of REQUIRED) if (m[k] === undefined || m[k] === "") bad(`${where}: '${k}' is missing`);
  if (ids.has(m.id)) bad(`${where}: duplicate id`);
  ids.add(m.id);
  if (!sha.test(m.sha256 ?? "")) bad(`${where}: sha256 is not 64 lower-case hex characters`);
  if (!Number.isInteger(m.bytes) || m.bytes <= 0) bad(`${where}: bytes must be a positive integer`);
  if (typeof m.file === "string" && (m.file.startsWith("/") || m.file.includes(":"))) bad(`${where}: file must be relative to models/`);
  const s = m.source ?? {};
  if (!s.repo || !/^[0-9a-f]{12,40}$/.test(s.revision ?? "")) bad(`${where}: source needs a repo and an exact revision (hex commit)`);
  for (const k of Object.keys(s)) if (k !== "repo" && k !== "revision") bad(`${where}: source key '${k}' is not allowed`);
  if (m.conversion) {
    for (const k of Object.keys(m.conversion)) if (!CONVERSION_KEYS.has(k)) bad(`${where}: conversion key '${k}' is not allowed`);
    if (m.conversion.deterministic === undefined || m.conversion.deterministic === null) bad(`${where}: conversion.deterministic must be true or false (measured)`);
    if (!sha.test(m.conversion.input_sha256 ?? "")) {
      // A conversion from a checkpoint folder has no single input file; the script then names the source revision.
      if (m.conversion.input_sha256) bad(`${where}: conversion.input_sha256 is not a SHA-256`);
    }
  }
}

// No ignore rule may hide the manifest.
try {
  const out = execFileSync("git", ["check-ignore", "-v", FILE], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  if (out.trim()) bad(`a git ignore rule matches ${FILE}: ${out.trim()}`);
} catch {
  // exit status 1 = not ignored: what we want
}

if (problems.length) {
  console.error(`${FILE}: ${problems.length} problem(s)`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log(`${FILE}: ok (${doc.models.length} models, metadata only, not ignored)`);
