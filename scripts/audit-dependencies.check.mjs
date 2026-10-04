import assert from "node:assert/strict";
import { test } from "node:test";
import { reviewAudit } from "./audit-dependencies.mjs";

const issue = {
  type: "auditAdvisory",
  data: {
    advisory: {
      github_advisory_id: "GHSA-vfj7-8cjw-p6xm",
      module_name: "braces",
      findings: [{ version: "3.0.3" }],
      severity: "high",
    },
    resolution: { path: "tailwindcss>chokidar>braces" },
  },
};
const report = (finding = issue) =>
  [finding, { type: "auditSummary", data: { vulnerabilities: { high: 1 } } }]
    .map((item) => JSON.stringify(item))
    .join("\n");

test("the known exception is restricted to the reviewed website path and expires", () => {
  const now = new Date("2026-10-04");
  assert.equal(reviewAudit(report(), "website", now).accepted.length, 1);
  assert.equal(reviewAudit(report(), ".", now).outstanding.length, 1);
  assert.equal(reviewAudit(report(), "website", new Date("2027-01-01")).outstanding.length, 1);
  const other = structuredClone(issue);
  other.data.resolution.path = "production-library>braces";
  assert.equal(reviewAudit(report(other), "website", now).outstanding.length, 1);
  other.data.advisory.github_advisory_id = "GHSA-new-issue";
  assert.equal(reviewAudit(report(other), "website", now).outstanding.length, 1);
});

test("registry errors, malformed output and incomplete reports fail closed", () => {
  for (const output of [
    "not JSON",
    JSON.stringify(issue),
    JSON.stringify({ type: "error", data: "Registry unavailable" }),
    JSON.stringify({ type: "auditSummary", data: { vulnerabilities: { high: 1 } } }),
  ]) {
    assert.throws(() => reviewAudit(output, "website"));
  }
});

test("a clean report needs no exceptions", () => {
  const output = JSON.stringify({ type: "auditSummary", data: { vulnerabilities: { high: 0 } } });
  assert.deepEqual(reviewAudit(output, "."), { outstanding: [], accepted: [] });
});
