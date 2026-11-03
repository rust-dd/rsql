import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = mkdtempSync(join(tmpdir(), "rsql-icons-"));
const destination = join(root, "src-tauri", "icons");

try {
  const result = spawnSync(
    process.execPath,
    [
      join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"),
      "icon",
      join(root, "assets", "app-icon.png"),
      "--output",
      output,
    ],
    { cwd: root, stdio: "inherit" },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Icon generation failed (${result.status}).`);

  mkdirSync(destination, { recursive: true });
  for (const entry of readdirSync(output, { withFileTypes: true })) {
    if (entry.isFile() && /\.(png|ico|icns)$/.test(entry.name)) {
      copyFileSync(join(output, entry.name), join(destination, entry.name));
    }
  }
} finally {
  rmSync(output, { recursive: true, force: true });
}
