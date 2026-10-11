#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

// Match only the retired product-name family. This deliberately does NOT match
// legitimate locality/business text such as "Amwaj".
const OLD_BRAND = /amwa[\s_-]*pos/gi;
const OLD_BRAND_PATH = /amwa[\s_-]*pos/i;
const AMWAJ = /\bamwaj\b/gi;

// Files below intentionally contain deployed compatibility identifiers. Any old
// product-name occurrence outside this narrow list is a regression. Keep this
// list explicit so adding a new compatibility surface requires code review.
const COMPATIBILITY_FILES = new Set([
  "crates/tillbh-core/src/channel.rs",
  "crates/tillbh-core/src/service.rs",
  "crates/tillbh-core/tests/rename_compat.rs",
  "scripts/upgrade-seed.mjs",
  "scripts/upgrade-smoke.ps1",
  "src-tauri/src/lib.rs",
  "src-tauri/src/secrets.rs",
  "src-tauri/tauri.windows.conf.json",
  "src-tauri/windows/hooks.nsh",
  "src/lib/legacyStorage.ts",
  "src/lib/__tests__/legacyStorage.test.ts",
]);

const listed = spawnSync("git", ["ls-files", "-z"], { encoding: "buffer" });
if (listed.status !== 0) {
  process.stderr.write(listed.stderr || Buffer.from("git ls-files failed\n"));
  process.exit(listed.status ?? 1);
}

const paths = listed.stdout
  .toString("utf8")
  .split("\0")
  .filter(Boolean);

let intentional = 0;
let amwaj = 0;
const accidental = [];

for (const path of paths) {
  if (OLD_BRAND_PATH.test(path)) {
    accidental.push(`${path}: tracked path contains retired product branding`);
  }

  const data = readFileSync(path);
  if (data.includes(0)) continue; // binary tracked blob
  const text = data.toString("utf8");
  amwaj += (text.match(AMWAJ) ?? []).length;

  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    const matches = lines[i].match(OLD_BRAND) ?? [];
    if (!matches.length) continue;
    if (COMPATIBILITY_FILES.has(path)) {
      intentional += matches.length;
    } else {
      accidental.push(`${path}:${i + 1}: ${lines[i].trim()}`);
    }
  }
}

console.log(`legacy-brand-audit accidental=${accidental.length} intentional_compatibility=${intentional} legitimate_amwaj=${amwaj}`);
if (accidental.length) {
  console.error("Unexpected retired-brand matches:");
  accidental.forEach((match) => console.error(`  ${match}`));
  process.exit(1);
}
