import { expect, it } from "vitest";
import { statementAt } from "./statement";

function at(marked: string) {
  const [before] = marked.split("|");
  const lines = before.split("\n");
  return statementAt(marked.replace("|", ""), {
    lineNumber: lines.length,
    column: lines[lines.length - 1].length + 1,
  });
}

it.each([
  "SELECT ';'; SELECT u.| FROM users u; SELECT 1",
  'SELECT "semi;colon"; SELECT u.| FROM users u; SELECT 1',
  "DO $tag$ BEGIN RAISE NOTICE ';'; END $tag$; SELECT u.| FROM users u; SELECT 1",
  "SELECT E'\\';'; SELECT u.| FROM users u; SELECT 1",
  "SELECT 'it''s;fine'; SELECT u.| FROM users u; SELECT 1",
  "SELECT 1 /* nested /* ; */ ; */; SELECT u.| FROM users u; SELECT 1",
  "SELECT 1 -- ;\n; SELECT u.| FROM users u; SELECT 1",
])("ignores terminators inside quotes and comments: %s", (sql) => {
  const statement = at(sql);
  expect(statement.sql).toBe(" SELECT u. FROM users u");
  expect(statement.caret).toEqual({ lineNumber: 1, column: 11 });
});

it("keeps nested queries and CTEs together", () => {
  expect(at("SELECT 1; WITH recent AS (SELECT * FROM users) SELECT r.| FROM recent r").sql).toBe(
    " WITH recent AS (SELECT * FROM users) SELECT r. FROM recent r",
  );
});

it("maps replacement ranges back to the original line and column", () => {
  const statement = at("SELECT 1;\nSELECT 2; SELECT u.| FROM users u");
  const global = { startLineNumber: 2, endLineNumber: 2, startColumn: 20, endColumn: 20 };
  expect(statement.globalRange(statement.localRange(global))).toEqual(global);
  expect(statement.caret).toEqual({ lineNumber: 1, column: 11 });
});

it("retains the full context for unquoted SQL function bodies", () => {
  const sql = "CREATE FUNCTION f() RETURNS int LANGUAGE SQL BEGIN ATOMIC SELECT 1; SELECT |2; END";
  expect(at(sql).sql).toBe(sql.replace("|", ""));
});
