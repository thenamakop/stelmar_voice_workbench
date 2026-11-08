// Fails when committed evidence (docs/evidence/ by default) contains personal or secret data: the
// repository is public. Writers redact through scripts/lib/Redact.psm1; this is the backstop.
// Text files are read as UTF-8, other files (PNG) as latin1 so metadata strings still show.
//
// Usage: node scripts/check-evidence.mjs [file or dir ...]   (default: docs/evidence)
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const RULES = [
  ["environment block", /Environment:/],
  // Written with a character class so `git grep` for the literal strings finds only real leaks.
  ["Claude Code variable", /CLAUDE_COD[E]_/],
  ["computer name variable", /COMPUTERNAM[E]=/i],
  ["user name variable", /USERNAME=/i],
  // A user folder by name; %USERPROFILE% and the shared folders are fine.
  ["user folder", /\\Users\\(?!(?:Public|Default|All Users)(?:\\|$))[^\\%\s"'<>|]+/i],
  ["secret assignment", /\b[A-Za-z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD)=(?!<redacted>)/i],
  ["GitHub token", /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|\bgithub_pat_[A-Za-z0-9_]{20,}/],
  ["API key", /\bsk-(?:ant-)?[A-Za-z0-9_-]{20,}/],
];
// Run locally, also look for this machine's own names (in CI these are the runner's, harmless).
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
for (const [name, value] of [
  ["computer name", process.env.COMPUTERNAME],
  ["user name", process.env.USERNAME],
]) {
  if (value && value.length >= 4) RULES.push([name, new RegExp(`\\b${escape(value)}\\b`, "i")]);
}
const TEXT = /\.(txt|log|md|json|csv|html|ps1|psm1)$/i;

const targets = process.argv.slice(2);
if (targets.length === 0) targets.push("docs/evidence");

function* walk(path) {
  if (statSync(path).isDirectory()) {
    for (const name of readdirSync(path)) yield* walk(join(path, name));
  } else yield path;
}

const problems = [];
let scanned = 0;
for (const target of targets) {
  if (!existsSync(target)) continue;
  for (const file of walk(target)) {
    scanned += 1;
    const lines = readFileSync(file, TEXT.test(file) ? "utf8" : "latin1").split(/\r?\n/);
    lines.forEach((line, i) => {
      for (const [name, pattern] of RULES) {
        // Report where, never what: the matched text may be the secret itself (and CI logs are public).
        const m = pattern.exec(line);
        if (m) problems.push(`${relative(process.cwd(), file)}:${i + 1}:${m.index + 1}: ${name}`);
      }
    });
  }
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(
    `\ncheck-evidence: personal or secret data in ${problems.length} place(s); ` +
      "run scripts/redact-evidence.ps1 on the file(s), or remove them",
  );
  process.exit(1);
}
console.log(`check-evidence: ${scanned} files, no personal or secret data found`);
