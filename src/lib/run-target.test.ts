import { describe, expect, it } from "vitest";
import { runTarget } from "./sql-statements";

const caret = (sql: string, needle: string, delta = 0) => {
  const at = sql.indexOf(needle) + delta;
  return { start: at, end: at };
};
const run = (sql: string, selection?: { start: number; end: number }) => {
  const target = runTarget(sql, selection);
  return "statement" in target ? target.statement.text : `error: ${target.error}`;
};

describe("runTarget", () => {
  const script = [
    "SELECT ';' AS semi, \"a;b\" FROM t; -- trailing; comment",
    "/* block; */ UPDATE t SET note = $$x; y$$;",
    "CREATE FUNCTION f() RETURNS int LANGUAGE sql",
    "BEGIN ATOMIC",
    "  SELECT 1;",
    "  SELECT CASE WHEN true THEN 2 END;",
    "END;",
    "SELECT 3",
  ].join("\n");

  it("runs the statement at the caret through quotes, comments and dollar quotes", () => {
    expect(run(script, caret(script, "semi"))).toBe("SELECT ';' AS semi, \"a;b\" FROM t");
    expect(run(script, caret(script, "UPDATE"))).toBe("UPDATE t SET note = $$x; y$$");
    expect(run(script, caret(script, "SELECT 3", 3))).toBe("SELECT 3");
  });

  it("runs a whole BEGIN ATOMIC routine from a caret inside its body", () => {
    const routine = run(script, caret(script, "SELECT 1;", 3));
    expect(routine.startsWith("CREATE FUNCTION f()")).toBe(true);
    expect(routine.endsWith("END")).toBe(true);
  });

  it("runs the selection as written, even when it holds several statements", () => {
    const from = script.indexOf("SELECT 3") - "END;\n".length;
    expect(run(script, { start: from, end: script.length })).toBe("END;\nSELECT 3");
    const both = { start: 0, end: script.indexOf("CREATE") };
    expect(run(script, both)).toBe(script.slice(0, script.indexOf("CREATE")).trim());
  });

  it("runs only a selected fragment, never the editor, when the fragment is invalid SQL", () => {
    const sql = "SELECT 1;\nSELECT FROM WHERE;\nDROP TABLE t;";
    const start = sql.indexOf("SELECT FROM");
    expect(run(sql, { start, end: start + "SELECT FROM WHERE".length })).toBe("SELECT FROM WHERE");
  });

  it("treats an empty selection as the caret and rejects one without SQL", () => {
    expect(run("SELECT 1;\nSELECT 2;", caret("SELECT 1;\nSELECT 2;", "2"))).toBe("SELECT 2");
    expect(run("SELECT 1; -- note", { start: 10, end: 17 })).toMatch(/no SQL statement/);
    expect(run("SELECT 1;\n   \nSELECT 2;", { start: 10, end: 13 })).toMatch(/no SQL statement/);
  });

  it("points to Run script when the caret is between statements", () => {
    const sql = "SELECT 1;\n\nSELECT 2;";
    expect(run(sql, { start: 10, end: 10 })).toMatch(/several statements.*Run script/);
    expect(run("-- nothing\n")).toMatch(/no SQL statement to run/);
  });
});
