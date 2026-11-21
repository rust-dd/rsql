#!/usr/bin/env node

// Writes DIR/inventory.json, the payload signed into native-latest.json, from release assets and their .sig files.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    repo: { type: "string" },
    tag: { type: "string" },
    revision: { type: "string" },
    "macos-minimum": { type: "string" },
  },
});

const [dir] = positionals;
const { repo, tag, revision } = values;
if (!dir || !repo || !tag || !revision || !values["macos-minimum"]) {
  console.error(
    "usage: native-inventory.mjs DIR --repo R --tag T --revision SHA --macos-minimum V",
  );
  process.exit(1);
}

const version = tag.replace(/^v/, "");
const prerelease = version.split("+")[0].split("-").slice(1).join("-");
const floor = (text) => [...text.split("."), "0", "0"].slice(0, 3).join(".");
const macos = floor(values["macos-minimum"]);
// Windows reports 10.x for both Windows 10 and 11; Rust's std needs Linux 3.2 or later.
const windows = "10.0.0";
const linux = "3.2.0";

const kinds = [
  ["_aarch64.app.tar.gz", "aarch64-apple-darwin", "app", "app-tar-gz", macos],
  ["-setup.exe", "x86_64-pc-windows-msvc", "nsis", "nsis", windows],
  [".AppImage", "x86_64-unknown-linux-gnu", "appimage", "appimage", linux],
  [".deb", "x86_64-unknown-linux-gnu", "deb", "deb", linux],
  [".rpm", "x86_64-unknown-linux-gnu", "rpm", "rpm", linux],
];

const artifacts = [];
for (const [suffix, target, installation, format, minimum] of kinds) {
  const names = readdirSync(dir).filter((name) => name.endsWith(suffix));
  if (names.length !== 1) {
    console.error(`Expected one ${suffix} asset in ${dir}, found ${names.length}.`);
    process.exit(1);
  }
  const [name] = names;
  const path = join(dir, name);
  artifacts.push({
    target,
    installation,
    format,
    minimum_os: minimum,
    url: `https://github.com/${repo}/releases/download/${tag}/${name}`,
    bytes: statSync(path).size,
    sha256: createHash("sha256").update(readFileSync(path)).digest("hex"),
    signature: readFileSync(`${path}.sig`, "utf8").trim(),
  });
}

const inventory = {
  format: 1,
  app: "com.rust-dd.rsql",
  channel: prerelease ? prerelease.split(".")[0] : "stable",
  version,
  revision,
  notes: "See the assets to download and install this version.",
  date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  artifacts,
};

writeFileSync(join(dir, "inventory.json"), `${JSON.stringify(inventory, null, 2)}\n`);
console.log(`Wrote ${artifacts.length} artifacts to ${join(dir, "inventory.json")}.`);
