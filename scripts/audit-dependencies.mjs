import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export function reviewAudit(output, directory, today = new Date()) {
  const findings = new Map();
  let summary;
  for (const line of output.trim().split("\n")) {
    const item = JSON.parse(line);
    if (item.type === "error") throw new Error(item.data);
    if (item.type === "auditSummary") summary = item.data;
    if (item.type === "auditAdvisory") {
      const { advisory, resolution } = item.data;
      findings.set(`${advisory.github_advisory_id}:${resolution.path}`, item.data);
    }
  }
  if (!summary) throw new Error("The registry did not return a complete audit report");
  const outstanding = [];
  const accepted = [];
  for (const { advisory, resolution } of findings.values()) {
    // Tailwind 3 supplies these build-time patterns from checked-in configuration.
    // No fix exists yet: https://github.com/advisories/GHSA-vfj7-8cjw-p6xm
    const knownBuildOnlyIssue =
      directory === "website" &&
      advisory.github_advisory_id === "GHSA-vfj7-8cjw-p6xm" &&
      advisory.module_name === "braces" &&
      advisory.findings.every(({ version }) => version === "3.0.3") &&
      ["tailwindcss>chokidar>braces", "tailwindcss>fast-glob>micromatch>braces"].includes(
        resolution.path,
      ) &&
      today < new Date("2026-12-31T00:00:00Z");
    (knownBuildOnlyIssue ? accepted : outstanding).push({
      id: advisory.github_advisory_id,
      path: resolution.path,
      severity: advisory.severity,
      title: advisory.title,
    });
  }
  const count = Object.values(summary.vulnerabilities).reduce((sum, value) => sum + value, 0);
  if (count > 0 && findings.size === 0) throw new Error("Audit findings are missing");
  return { outstanding, accepted };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const directory = process.argv[2] ?? ".";
  if (![".", "website"].includes(directory)) throw new Error("Unknown package directory");
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const result = spawnSync("yarn", ["audit", "--json"], {
    cwd: resolve(root, directory),
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    timeout: 120_000,
  });
  if (result.error) throw result.error;
  if (result.signal || result.status === null) throw new Error("Audit process did not finish");
  const { outstanding, accepted } = reviewAudit(result.stdout, directory);
  for (const issue of accepted) {
    console.warn(`Known build-only exception (expires 2026-12-31): ${issue.id} ${issue.path}`);
  }
  for (const issue of outstanding) console.error(issue);
  console.log(
    `${directory}: ${outstanding.length} unaccepted findings, ${accepted.length} exceptions`,
  );
  process.exitCode = outstanding.length || (result.status !== 0 && accepted.length === 0) ? 1 : 0;
}
