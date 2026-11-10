// Checks the vendored whisper.cpp tree (third_party/whisper.cpp) against the hash pinned in
// third_party/README.md, so a changed, missing or extra file fails CI.
//
// Hash algorithm (the same in this script and in crates/wb-engine/build.rs): SHA-256 over, for each
// file under the tree in ordinal byte order of its relative path (separator "/"): the path bytes,
// one 0x00 byte, the file length as 8 bytes little-endian, then the file bytes.
//
// Usage:
//   node scripts/check-vendor.mjs            verify against third_party/README.md
//   node scripts/check-vendor.mjs --print    print the hash and the file count
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

const TREE = join("third_party", "whisper.cpp");
const README = join("third_party", "README.md");

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) yield* walk(p);
    else yield p;
  }
}

export function treeHash(root) {
  const files = [...walk(root)].map((p) => relative(root, p).split(sep).join("/"));
  // Ordinal order of the UTF-8 bytes of the path.
  files.sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const h = createHash("sha256");
  for (const rel of files) {
    const data = readFileSync(join(root, ...rel.split("/")));
    const len = Buffer.alloc(8);
    len.writeBigUInt64LE(BigInt(data.length));
    h.update(Buffer.from(rel));
    h.update(Buffer.from([0]));
    h.update(len);
    h.update(data);
  }
  return { hash: h.digest("hex"), files: files.length };
}

if (import.meta.url === `file://${process.argv[1].split(sep).join("/")}` || process.argv[1]?.endsWith("check-vendor.mjs")) {
  if (!existsSync(TREE)) {
    console.error(`check-vendor: ${TREE} does not exist`);
    process.exit(1);
  }
  const { hash, files } = treeHash(TREE);
  if (process.argv.includes("--print")) {
    console.log(`tree-sha256: ${hash}`);
    console.log(`files: ${files}`);
    process.exit(0);
  }
  const readme = readFileSync(README, "utf8");
  const pinned = /^tree-sha256:\s*([0-9a-f]{64})\s*$/m.exec(readme)?.[1];
  const pinnedFiles = /^files:\s*(\d+)\s*$/m.exec(readme)?.[1];
  if (!pinned) {
    console.error(`check-vendor: ${README} has no "tree-sha256: <hex>" line`);
    process.exit(1);
  }
  if (pinned !== hash || (pinnedFiles && Number(pinnedFiles) !== files)) {
    console.error(
      `check-vendor: ${TREE} does not match the pin in ${README}\n  pinned   ${pinned} (${pinnedFiles ?? "?"} files)\n  computed ${hash} (${files} files)`,
    );
    process.exit(1);
  }
  console.log(`check-vendor: ${TREE} matches the pin (${files} files, ${hash.slice(0, 12)}...)`);
}
