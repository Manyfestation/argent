#!/usr/bin/env node
// Copy the canonical grammar queries into the Zed extension. The Zed tests
// fail when the copies drift, so run this after editing anything in queries/.

import { copyFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const treeSitterDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(treeSitterDir, "..");
const zedLanguageDir = path.join(repoRoot, "zed", "argent", "languages", "argent");

for (const query of ["highlights.scm", "indents.scm"]) {
  const target = path.join(zedLanguageDir, query);
  await mkdir(path.dirname(target), { recursive: true });
  await copyFile(path.join(treeSitterDir, "queries", query), target);
  console.log(`synced ${path.relative(repoRoot, target)}`);
}
