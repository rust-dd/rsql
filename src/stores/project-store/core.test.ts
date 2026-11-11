import { describe, expect, it } from "vitest";
import { buildProjectMap, projectName, toDetailArray } from "./core";

const row = (database: string, name: string, legacyId = "", serverAlias = "") => [
  "PGSQL",
  "app",
  "secret",
  database,
  "db.local",
  "5432",
  "false",
  "false",
  "",
  "22",
  "",
  "",
  "",
  name,
  serverAlias,
  legacyId,
];

describe("buildProjectMap", () => {
  it("orders connections by name rather than by key", () => {
    const { projects } = buildProjectMap({
      "a-key": row("users", "beta"),
      "b-key": row("orders", "Alpha"),
    });
    expect(Object.keys(projects)).toEqual(["b-key", "a-key"]);
  });

  it("reads the name and maps each legacy key to its new key", () => {
    const { projects, legacyIds } = buildProjectMap({
      "uuid-1": row("orders", "prod", "prod"),
      "uuid-2": row("users", "fresh"),
    });
    expect(projects["uuid-1"]?.name).toBe("prod");
    expect(legacyIds).toEqual(new Map([["prod", "uuid-1"]]));
  });
});

describe("toDetailArray", () => {
  it("sends the name and server alias right after the SSH fields", () => {
    const { projects } = buildProjectMap({ k: row("orders", "prod", "prod", "Railway") });
    const details = projects.k;
    if (!details) throw new Error("project missing");
    expect(projects.k?.serverAlias).toBe("Railway");
    expect(toDetailArray(details)).toEqual(row("orders", "prod", "", "Railway").slice(0, 15));
  });
});

describe("projectName", () => {
  it("prefers the name and falls back to the key", () => {
    const { projects } = buildProjectMap({ k: row("orders", "prod"), blank: row("orders", "") });
    expect(projectName(projects, "k")).toBe("prod");
    expect(projectName(projects, "blank")).toBe("blank");
    expect(projectName(projects, "missing")).toBe("missing");
  });
});
