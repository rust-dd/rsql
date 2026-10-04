import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "rsql-version-check-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, "scripts"));
  mkdirSync(join(root, "src-tauri"));
  copyFileSync(
    new URL("./check-version.mjs", import.meta.url),
    join(root, "scripts/check-version.mjs"),
  );
  writeFileSync(join(root, "package.json"), '{"version":"1.3.0"}');
  writeFileSync(join(root, "src-tauri/tauri.conf.json"), '{"version":"1.3.0"}');
  writeFileSync(
    join(root, "src-tauri/Cargo.toml"),
    '[package]\nname = "rsql"\nversion = "1.3.0"\n',
  );
  writeFileSync(
    join(root, "src-tauri/Cargo.lock"),
    '[[package]]\nname = "rsql"\nversion = "1.3.0"\n',
  );
  const run = (...args) =>
    spawnSync(process.execPath, ["scripts/check-version.mjs", ...args], {
      cwd: root,
      encoding: "utf8",
    });
  const git = (...args) =>
    execFileSync(
      "git",
      [
        "-c",
        "user.name=Version Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        ...args,
      ],
      { cwd: root, stdio: "pipe" },
    );
  return { root, run, git };
}

test("version setting updates manifests and only the application lockfile entry", (t) => {
  const { root, run } = fixture(t);
  const lock = join(root, "src-tauri/Cargo.lock");
  writeFileSync(
    lock,
    `${readFileSync(lock, "utf8")}\n[[package]]\nname = "other"\nversion = "9.9.9"\n`,
  );
  assert.equal(run("--set", "1.3.1").status, 0);
  assert.equal(run().status, 0);
  assert.match(readFileSync(lock, "utf8"), /name = "other"\nversion = "9.9.9"/);
  writeFileSync(join(root, "package.json"), '{"version":"1.2.0"}');
  assert.equal(run().status, 1);
});

test("missing lockfile version and malformed version requests fail", (t) => {
  const { root, run } = fixture(t);
  assert.equal(run("--set").status, 1);
  assert.equal(run("--set", "invalid").status, 1);
  writeFileSync(join(root, "src-tauri/Cargo.lock"), "");
  assert.equal(run().status, 1);
});

test("Windows checkouts preserve CRLF while reading and updating the lockfile", (t) => {
  const { root, run } = fixture(t);
  const path = join(root, "src-tauri/Cargo.lock");
  writeFileSync(path, readFileSync(path, "utf8").replaceAll("\n", "\r\n"));
  assert.equal(run().status, 0);
  assert.equal(run("--set", "1.3.1").status, 0);
  assert.equal(run().status, 0);
  assert.match(readFileSync(path, "utf8"), /name = "rsql"\r\nversion = "1.3.1"/);
});

test("a release tag must match the version and the exact checked-out commit", (t) => {
  const { run, git } = fixture(t);
  git("init");
  git("commit", "--allow-empty", "-m", "fixture");
  assert.equal(run("--tag", "v1.2.0").status, 1);
  assert.notEqual(run("--tag", "v1.3.0").status, 0);
  git("tag", "-a", "v1.3.0", "-m", "fixture release");
  assert.equal(run("--tag", "v1.3.0").status, 0);
  git("commit", "--allow-empty", "-m", "different commit");
  assert.equal(run("--tag", "v1.3.0").status, 1);
});
