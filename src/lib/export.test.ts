import { beforeEach, describe, expect, it, vi } from "vitest";
import { exportResults, toCSV, toJSON, toSQL, toXML } from "./export";

const { save, writeTextFile } = vi.hoisted(() => ({ save: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile }));
beforeEach(() => vi.clearAllMocks());

describe("faithful exports", () => {
  it("quotes CR/LF and distinguishes CSV NULL from empty strings", () => {
    expect(
      toCSV(
        ["a\rb", "value"],
        [
          [null, ""],
          ['x\ry\n"z', "null"],
        ],
      ),
    ).toBe('"a\rb",value\n,""\n"x\ry\n""z",null');
  });
  it("rejects duplicate JSON aliases instead of losing columns", () => {
    expect(() => toJSON(["id", "id"], [["1", "2"]])).toThrow("unique column names");
  });
  it("preserves prototype-like keys and typed nulls in JSON", () => {
    const text = toJSON(
      ["__proto__", "constructor", "nil", "empty"],
      [["first", "second", null, ""]],
    );
    expect(JSON.parse(text)).toEqual([
      JSON.parse('{"__proto__":"first","constructor":"second","nil":null,"empty":""}'),
    ]);
  });
  it("escapes SQL identifiers independently from string literals", () => {
    expect(toSQL(['a"b', "nil"], [["O'Reilly", null]], 't"x')).toBe(
      'INSERT INTO "t""x" ("a""b", "nil") VALUES (\'O\'\'Reilly\', NULL);',
    );
  });
  it("supports arbitrary and repeated XML names with explicit nulls", () => {
    const text = toXML(['1 bad"<&', "value", "value"], [["a\rb", null, ""]]);
    expect(text).toContain('<column name="1 bad&quot;&lt;&amp;">a&#13;b</column>');
    expect(text).toContain('<column name="value" null="true" />');
    expect(text).toContain('<column name="value"></column>');
  });
  it("refuses XML-invalid controls without changing data", () => {
    expect(() => toXML(["value"], [["\x1f"]])).toThrow("XML cannot represent");
  });
  it("does not write a file when its dialog is cancelled", async () => {
    save.mockResolvedValueOnce(null);
    expect(await exportResults("csv", ["value"], [["1"]])).toBe(false);
    expect(writeTextFile).not.toHaveBeenCalled();
  });
  it("propagates file errors to the UI", async () => {
    save.mockResolvedValueOnce("/export.csv");
    writeTextFile.mockRejectedValueOnce(new Error("disk full"));
    await expect(exportResults("csv", ["value"], [["1"]])).rejects.toThrow("disk full");
  });
});
