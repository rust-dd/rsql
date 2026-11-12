import { describe, expect, it } from "vitest";
import { splitStatements, targetStatement } from "./sql-statements";

const texts = (sql: string) => splitStatements(sql).map((s) => s.text);

describe("splitStatements", () => {
  it("splits on top-level semicolons and drops empty statements", () => {
    expect(texts("SELECT 1; ;\n SELECT 2;;")).toEqual(["SELECT 1", "SELECT 2"]);
  });

  it("keeps a final statement without a semicolon", () => {
    expect(texts("SELECT 1;\nSELECT 2\n")).toEqual(["SELECT 1", "SELECT 2"]);
  });

  it("ignores semicolons in string literals and quoted identifiers", () => {
    expect(texts(`SELECT 'a;b', 'it''s;' FROM "we;ird""t"; SELECT 2`)).toEqual([
      `SELECT 'a;b', 'it''s;' FROM "we;ird""t"`,
      "SELECT 2",
    ]);
  });

  it("treats backslashes as escapes only in E strings", () => {
    expect(texts(String.raw`SELECT E'\';'; SELECT 'C:\'; SELECT 3`)).toEqual([
      String.raw`SELECT E'\';'`,
      String.raw`SELECT 'C:\'`,
      "SELECT 3",
    ]);
    expect(texts(String.raw`SELECT some'x\'; SELECT 2`)).toHaveLength(2);
  });

  it("ignores semicolons in line and nested block comments", () => {
    const sql = "SELECT 1 -- no; split\n, /* outer /* inner; */ still; */ 2;\nSELECT 3";
    expect(texts(sql)).toEqual([
      "SELECT 1 -- no; split\n, /* outer /* inner; */ still; */ 2",
      "SELECT 3",
    ]);
  });

  it("excludes leading and trailing comments from a statement", () => {
    expect(texts("-- header\n/* c */ SELECT 1 -- tail\n; -- after")).toEqual(["SELECT 1"]);
    expect(texts("-- only a comment;\n/* and; another */")).toEqual([]);
  });

  it("ignores semicolons in dollar-quoted bodies", () => {
    const fn =
      "CREATE FUNCTION f() RETURNS int AS $body$ BEGIN RETURN 1; END; $body$ LANGUAGE plpgsql";
    expect(texts(`${fn}; SELECT $$a;b$$; SELECT 2`)).toEqual([fn, "SELECT $$a;b$$", "SELECT 2"]);
  });

  it("does not mistake parameters or identifiers with dollars for dollar quotes", () => {
    expect(texts("SELECT $1, a$b$c; SELECT 2")).toEqual(["SELECT $1, a$b$c", "SELECT 2"]);
  });

  it("accepts non-ASCII identifiers and dollar-quote tags", () => {
    expect(texts("SELECT 1 AS évszám; SELECT $tág$a;b$tág$; SELECT 2")).toEqual([
      "SELECT 1 AS évszám",
      "SELECT $tág$a;b$tág$",
      "SELECT 2",
    ]);
  });

  it("ignores semicolons inside parentheses", () => {
    expect(texts("SELECT (1; 2); SELECT 3")).toEqual(["SELECT (1; 2)", "SELECT 3"]);
  });

  it("keeps BEGIN ATOMIC bodies of routines together, including CASE ... END", () => {
    const fn = [
      "CREATE OR REPLACE FUNCTION grade(n int) RETURNS text LANGUAGE sql",
      "BEGIN ATOMIC",
      "  SELECT CASE WHEN n > 1 THEN 'high' ELSE 'low' END;",
      "  SELECT 'done';",
      "END",
    ].join("\n");
    const proc = "CREATE PROCEDURE p() BEGIN ATOMIC INSERT INTO t VALUES (1); END";
    expect(texts(`${fn};\n${proc};\nSELECT 1`)).toEqual([fn, proc, "SELECT 1"]);
  });

  it("does not treat transaction BEGIN as a routine body", () => {
    expect(texts("BEGIN; UPDATE t SET a = 1; END;")).toEqual([
      "BEGIN",
      "UPDATE t SET a = 1",
      "END",
    ]);
  });

  it("records offsets and the terminator", () => {
    const [first, second] = splitStatements("  SELECT 1 ;\nSELECT 2");
    expect(first).toEqual({ text: "SELECT 1", start: 2, end: 10, terminator: 11 });
    expect(second).toEqual({ text: "SELECT 2", start: 13, end: 21, terminator: undefined });
  });

  it("consumes unterminated literals and comments to the end", () => {
    expect(texts("SELECT 'open; SELECT 2")).toEqual(["SELECT 'open; SELECT 2"]);
    expect(texts("SELECT 1 /* open; SELECT 2")).toEqual(["SELECT 1"]);
    expect(texts("SELECT $x$ open; SELECT 2")).toEqual(["SELECT $x$ open; SELECT 2"]);
  });
});

describe("targetStatement", () => {
  const script = "SELECT 1;\nDELETE FROM t;\n\nUPDATE t SET a = ';'";
  const at = (needle: string, delta = 0) => script.indexOf(needle) + delta;
  const text = (target: ReturnType<typeof targetStatement>) =>
    "statement" in target ? target.statement.text : target.error;

  it("uses the only statement wherever the caret is", () => {
    expect(text(targetStatement("\n\nSELECT 1;\n\n", { start: 0, end: 0 }))).toBe("SELECT 1");
    expect(text(targetStatement("SELECT 1"))).toBe("SELECT 1");
  });

  it("uses the statement at the caret, including right after its semicolon", () => {
    expect(text(targetStatement(script, { start: at("DELETE", 3), end: at("DELETE", 3) }))).toBe(
      "DELETE FROM t",
    );
    const afterSemicolon = at("SELECT 1;", 9);
    expect(text(targetStatement(script, { start: afterSemicolon, end: afterSemicolon }))).toBe(
      "SELECT 1",
    );
    const end = script.length;
    expect(text(targetStatement(script, { start: end, end }))).toBe("UPDATE t SET a = ';'");
  });

  it("rejects a caret between statements or a missing caret", () => {
    const blank = at("\n\nUPDATE", 1);
    expect(text(targetStatement(script, { start: blank, end: blank }))).toMatch(
      /several statements/,
    );
    expect(text(targetStatement(script))).toMatch(/several statements/);
  });

  it("uses exactly one selected statement and reports its offsets in the script", () => {
    const target = targetStatement(script, { start: at("DELETE"), end: at("DELETE FROM t;", 14) });
    expect(target).toEqual({
      statement: {
        text: "DELETE FROM t",
        start: at("DELETE"),
        end: at("DELETE", 13),
        terminator: at("DELETE", 13),
      },
    });
  });

  it("accepts a backwards selection", () => {
    expect(text(targetStatement(script, { start: at("SELECT 1", 8), end: 0 }))).toBe("SELECT 1");
  });

  it("rejects selections with several or no statements", () => {
    expect(text(targetStatement(script, { start: 0, end: at("UPDATE") }))).toMatch(/2 statements/);
    expect(text(targetStatement("SELECT 1; -- note", { start: 10, end: 17 }))).toMatch(
      /no SQL statement/,
    );
  });

  it("rejects an editor without statements", () => {
    expect(text(targetStatement("-- nothing here\n"))).toMatch(/no SQL statement to run/);
  });
});
