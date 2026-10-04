/**
 * The real parser wired to the real completion core, with only Monaco and the
 * database replaced.
 *
 * `build.test.ts` proves the core decides correctly given an input, and
 * `parser-contract.test.ts` proves the parser reports what we expect. Neither
 * catches the adapter in between misreading it, which is where a completion bug
 * would actually live.
 */

import { describe, expect, it } from "vitest";
import { buildCompletions } from "./build";
import { createCompletionParser } from "./parse";
import type { Catalog, IndexedColumn, IndexedRelation } from "./types";

function column(name: string, over: Partial<IndexedColumn> = {}): IndexedColumn {
  return {
    name,
    data_type: "text",
    nullable: true,
    default_value: null,
    is_primary_key: false,
    references: null,
    ...over,
  };
}

const PUBLIC: IndexedRelation[] = [
  {
    name: "users",
    kind: "table",
    comment: null,
    columns: [column("id", { data_type: "bigint" }), column("email")],
  },
  {
    name: "orders",
    kind: "table",
    comment: null,
    columns: [column("id", { data_type: "bigint" }), column("user_id", { data_type: "bigint" })],
  },
];

const CATALOG: Catalog = {
  defaultSchema: "public",
  schemas: () => ["public"],
  relations: () => PUBLIC,
  relation: (name) => PUBLIC.find((r) => r.name === name),
  functions: () => [],
};

/** Stand-in for Monaco's own idea of the word under the caret. */
function wordRangeAt(line: string, lineNumber: number, column: number) {
  const before = line.slice(0, column - 1);
  const word = /[A-Za-z0-9_]*$/.exec(before)?.[0] ?? "";
  return {
    startLineNumber: lineNumber,
    startColumn: column - word.length,
    endLineNumber: lineNumber,
    endColumn: column,
  };
}

/** `|` marks the caret. */
function complete(marked: string) {
  const offset = marked.indexOf("|");
  const sql = marked.replace("|", "");
  const before = sql.slice(0, offset);
  const lines = before.split("\n");
  const lineNumber = lines.length;
  const column = lines[lines.length - 1].length + 1;
  const position = { lineNumber, column };
  const wordRange = wordRangeAt(lines[lines.length - 1], lineNumber, column);

  const parsed = createCompletionParser()({
    document: "test",
    version: 1,
    sql,
    caret: position,
    wordRange,
  });

  return buildCompletions({
    expectation: parsed?.expectation ?? { kinds: [], qualifier: [], range: wordRange },
    keywords: parsed?.keywords ?? [],
    scope: parsed?.scope ?? [],
    catalog: CATALOG,
    snippets: [],
  });
}

const columns = (items: { kind: string; label: string }[]) =>
  items.filter((i) => i.kind === "column").map((i) => i.label);
const relations = (items: { kind: string; label: string }[]) =>
  items.filter((i) => i.kind === "relation").map((i) => i.label);

describe("parser to completions", () => {
  it("offers a table's own columns after its alias", () => {
    const items = complete("SELECT u.| FROM users u");
    expect(columns(items)).toEqual(["id", "email"]);
  });

  it("offers only the aliased table's columns, not both tables'", () => {
    const items = complete("SELECT o.| FROM users u JOIN orders o ON o.user_id = u.id");
    expect(columns(items)).toEqual(["id", "user_id"]);
  });

  it("offers columns of every table in scope when unqualified", () => {
    const items = complete("SELECT | FROM users u JOIN orders o ON o.user_id = u.id");
    expect(columns(items)).toEqual(["id", "email", "id", "user_id"]);
  });

  it("offers relations after FROM", () => {
    const items = complete("SELECT * FROM |");
    expect(relations(items)).toEqual(["users", "orders"]);
  });

  it("inserts only the table name after a schema qualifier", () => {
    const items = complete("SELECT * FROM public.|");
    const users = items.find((i) => i.label === "users");
    expect(users?.insertText).toBe("users ${1:u}");
  });

  it("resolves a schema-qualified table and its alias together", () => {
    const items = complete("SELECT u.| FROM public.users u");
    expect(columns(items)).toEqual(["id", "email"]);
  });

  it("replaces only the partially typed word", () => {
    const items = complete("SELECT * FROM us|");
    const users = items.find((i) => i.label === "users");
    expect(users?.range).toMatchObject({ startColumn: 15, endColumn: 17 });
  });

  it("ranks columns above the keywords the grammar also allows", () => {
    const items = complete("SELECT | FROM users u");
    expect(items.some((i) => i.kind === "keyword")).toBe(true);
    const sorted = [...items].sort((a, b) => a.sortText.localeCompare(b.sortText));
    expect(sorted[0].kind).toBe("column");
  });

  it("offers nothing after an alias that does not exist", () => {
    expect(complete("SELECT zz.| FROM users u")).toEqual([]);
  });

  it("does not offer a table's columns for a CTE of the same shape", () => {
    // `recent` is not in the catalog, so silence is correct here.
    expect(complete("WITH recent AS (SELECT * FROM orders) SELECT r.| FROM recent r")).toEqual([]);
  });
});

describe("typing the first word of a statement", () => {
  it("offers SELECT while it is being typed", () => {
    const items = complete("SEL|");
    expect(items.some((i) => i.label === "SELECT")).toBe(true);
  });

  it("replaces what was typed rather than inserting beside it", () => {
    // Without the editor's word range this collapsed to an empty span, so
    // accepting SELECT after typing SEL produced SELSELECT.
    const select = complete("SEL|").find((i) => i.label === "SELECT");
    expect(select?.range).toMatchObject({ startColumn: 1, endColumn: 4 });
  });

  it("still replaces correctly further into a line", () => {
    const select = complete("SELECT 1;\nSEL|").find((i) => i.label === "SELECT");
    expect(select?.range).toMatchObject({ startLineNumber: 2, startColumn: 1, endColumn: 4 });
  });

  it("keeps an empty span after a qualifier dot", () => {
    const items = complete("SELECT u.| FROM users u");
    expect(items[0]?.range).toMatchObject({ startColumn: 10, endColumn: 10 });
  });
});
