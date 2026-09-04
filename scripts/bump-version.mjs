#!/usr/bin/env node
// Bump version across every file that needs to stay in lockstep:
//   - package.json
//   - src-tauri/tauri.conf.json
//   - src-tauri/Cargo.toml + crates/*/Cargo.toml ([package].version)
//   - Cargo.lock (regenerated via `cargo update -w`)
//
// Usage: node scripts/bump-version.mjs <patch|minor|major|x.y.z>
//
// After this script, review the diff, commit with `chore: release vX.Y.Z`,
// then `git tag vX.Y.Z && git push origin main vX.Y.Z` to kick off the
// release workflow. (Not `--follow-tags`: it skips lightweight tags.)

import { readFileSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");

const arg = process.argv[2];
if (!arg) {
  console.error("usage: bump-version.mjs <patch|minor|major|x.y.z>");
  process.exit(1);
}

const pkg = JSON.parse(readFileSync(resolve(repoRoot, "package.json"), "utf8"));
const current = pkg.version;
const next = computeNext(current, arg);

console.log(`${current} -> ${next}`);

const cargoTomls = [
  "src-tauri/Cargo.toml",
  "crates/pg-core/Cargo.toml",
  "crates/pg-intellisense/Cargo.toml",
  "crates/pg-schema-cache/Cargo.toml",
  "crates/pg-profiles/Cargo.toml",
  "crates/pg-entra/Cargo.toml",
];

for (const path of cargoTomls) {
  const full = resolve(repoRoot, path);
  const content = readFileSync(full, "utf8");
  const updated = content.replace(
    /^version\s*=\s*"[^"]+"/m,
    `version = "${next}"`,
  );
  if (updated === content) {
    console.error(`[!] no [package].version line found in ${path}`);
    process.exit(1);
  }
  writeFileSync(full, updated);
}

pkg.version = next;
writeFileSync(
  resolve(repoRoot, "package.json"),
  JSON.stringify(pkg, null, 2) + "\n",
);

const tauriConfPath = resolve(repoRoot, "src-tauri/tauri.conf.json");
const tauriConf = JSON.parse(readFileSync(tauriConfPath, "utf8"));
tauriConf.version = next;
writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + "\n");

console.log("regenerating Cargo.lock...");
execSync("cargo update -w", { cwd: repoRoot, stdio: "inherit" });

console.log("");
console.log(`done. Review the diff, then:`);
console.log(`  git commit -am "chore: release v${next}"`);
console.log(`  git tag v${next}`);
console.log(`  git push origin main v${next}`);

function computeNext(curr, bump) {
  if (/^\d+\.\d+\.\d+/.test(bump)) return bump;
  const [maj, min, pat] = curr.split(".").map(Number);
  if (bump === "patch") return `${maj}.${min}.${pat + 1}`;
  if (bump === "minor") return `${maj}.${min + 1}.0`;
  if (bump === "major") return `${maj + 1}.0.0`;
  console.error(`unknown bump: ${bump}`);
  process.exit(1);
}
