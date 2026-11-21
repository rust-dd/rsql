#!/usr/bin/env node

// The native crate manifest and lockfile entry must match the release tag and commit.
// With --set <version> it aligns them instead of checking.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const cargoManifest = (path) => ({
  path,
  read: (text) => text.match(/^version\s*=\s*"([^"]+)"/m)?.[1],
  write: (text, version) => text.replace(/^(version\s*=\s*)"[^"]*"/m, `$1"${version}"`),
});

const lockEntry = (name) => {
  const pattern = new RegExp(
    `(\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = )"([^"]+)"`,
  );
  return {
    path: "Cargo.lock",
    read: (text) => text.match(pattern)?.[2],
    write: (text, version) => text.replace(pattern, `$1"${version}"`),
  };
};

const read = (source) => source.read(readFileSync(join(root, source.path), "utf8"));

const native = [cargoManifest("crates/rsql-desktop/Cargo.toml"), lockEntry("rsql-desktop")];
const setIndex = process.argv.indexOf("--set");
const target = setIndex === -1 ? null : process.argv[setIndex + 1];

if (setIndex !== -1 && !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(target ?? "")) {
  console.error(`Not a valid semver version: ${target ?? "(missing)"}`);
  process.exit(1);
}

if (target) {
  for (const source of native) {
    const file = join(root, source.path);
    writeFileSync(file, source.write(readFileSync(file, "utf8"), target));
    console.log(`${source.path} -> ${target}`);
  }
  process.exit(0);
}

function agreed(sources) {
  const found = sources.map((source) => ({ path: source.path, version: read(source) }));
  const missing = found.filter((entry) => !entry.version);
  if (missing.length > 0) {
    for (const entry of missing) console.error(`No version found in ${entry.path}`);
    process.exit(1);
  }
  const distinct = [...new Set(found.map((entry) => entry.version))];
  if (distinct.length > 1) {
    console.error("Version mismatch:");
    for (const entry of found) console.error(`  ${entry.path}: ${entry.version}`);
    process.exit(1);
  }
  return distinct[0];
}

const version = agreed(native);

const tagIndex = process.argv.indexOf("--tag");
if (tagIndex !== -1) {
  const tag = process.argv[tagIndex + 1];
  if (tag !== `v${version}`) {
    console.error(`Release tag ${tag ?? "(missing)"} does not match v${version}.`);
    process.exit(1);
  }
  const revision = (ref) =>
    execFileSync("git", ["rev-parse", "--verify", ref], { cwd: root, encoding: "utf8" }).trim();
  if (revision(`refs/tags/${tag}^{commit}`) !== revision("HEAD")) {
    console.error("Release tag does not point to the checked-out commit.");
    process.exit(1);
  }
}
console.log(`Version ${version} is consistent across the native manifest and the Rust lockfile.`);
