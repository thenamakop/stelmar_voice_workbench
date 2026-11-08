// Fails when an HTTP-client or networking crate is compiled into the Windows app or engines
// (PLAN.md section 10). Cargo.lock lists every target's dependencies (Tauri pulls reqwest for its
// mobile targets only), so this asks cargo for the normal dependency graph of the Windows target.
//
// Usage: node scripts/check-http-crates.mjs
import { execFileSync } from "node:child_process";

const TARGET = "x86_64-pc-windows-msvc";
const FORBIDDEN = [
  "reqwest",
  "hyper",
  "hyper-util",
  "ureq",
  "curl",
  "isahc",
  "attohttpc",
  "surf",
  "tokio-tungstenite",
  "tungstenite",
  "native-tls",
  "rustls",
  "h2",
  "quinn",
  "minreq",
  "async-tungstenite",
  "self_update",
  "tiny_http",
  "axum",
  "warp",
  // Raw sockets: absent today, so they flag the first dependency that brings real networking.
  "mio",
  "socket2",
];
// No Tauri plugins at all; crash reporters, telemetry and DNS resolvers by family name.
const FORBIDDEN_PREFIX = ["tauri-plugin-", "sentry", "opentelemetry", "hickory-", "trust-dns"];
// The windows crates reach WinHTTP, WinINet, WinSock and WinRT networking through features, not
// crate names (the app adds `windows` for DXGI, Job Objects and MMCSS in Phase 1).
const FORBIDDEN_WINDOWS_FEATURE =
  /^windows(?:-sys)? feature "(?:Win32_Networking|Web|Networking)(?:_[A-Za-z0-9_]*)?"/;

const tree = execFileSync(
  "cargo",
  ["tree", "--workspace", "-e", "normal", "--target", TARGET, "--prefix", "none", "--format", "{p}"],
  { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
);
const crates = new Set(
  tree
    .split(/\r?\n/)
    .map((line) => line.trim().split(" ")[0])
    .filter(Boolean),
);
const found = [...crates].filter(
  (name) => FORBIDDEN.includes(name) || FORBIDDEN_PREFIX.some((prefix) => name.startsWith(prefix)),
);

if (found.length) {
  console.error(`check-http-crates: networking crates in the ${TARGET} build: ${found.join(", ")}`);
  for (const name of found) {
    console.error(
      execFileSync("cargo", ["tree", "--workspace", "-e", "normal", "--target", TARGET, "-i", name], {
        encoding: "utf8",
      }),
    );
  }
  process.exit(1);
}

const features = execFileSync(
  "cargo",
  ["tree", "--workspace", "-e", "features", "--target", TARGET, "--prefix", "none"],
  { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
);
const badFeatures = [
  ...new Set(
    features
      .split(/\r?\n/)
      .map((line) => line.trim().replace(/ \(\*\)$/, ""))
      .filter((line) => FORBIDDEN_WINDOWS_FEATURE.test(line)),
  ),
];
if (badFeatures.length) {
  console.error(`check-http-crates: networking features of the windows crates in the ${TARGET} build:`);
  console.error(badFeatures.join("\n"));
  process.exit(1);
}
console.log(
  `check-http-crates: ${crates.size} crates in the ${TARGET} build, no networking crates or windows networking features`,
);
