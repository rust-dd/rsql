import { describe, expect, it } from "vitest";
import fixtures from "../../tests/fixtures/query-wire.json";
import {
  CELL_SEP,
  type CellValue,
  decodeCell,
  decodeColumns,
  decodePage,
  decodeResult,
  ESC,
  encodeCell,
  encodeResult,
  encodeRows,
  ROW_SEP,
} from "./wire";

function roundTrip(rows: CellValue[][]) {
  expect(decodePage(encodeRows(rows))).toEqual(rows);
}

describe("decodeCell", () => {
  it("distinguishes SQL NULL from the text null", () => {
    expect(decodeCell(`${ESC}N`)).toBeNull();
    expect(decodeCell("null")).toBe("null");
    expect(decodeCell("NULL")).toBe("NULL");
  });

  it("distinguishes SQL NULL from the empty string", () => {
    expect(decodeCell(`${ESC}N`)).toBeNull();
    expect(decodeCell("")).toBe("");
  });

  it("unescapes separators occurring in data", () => {
    expect(decodeCell(`a${ESC}Ab`)).toBe(`a${CELL_SEP}b`);
    expect(decodeCell(`a${ESC}Bb`)).toBe(`a${ROW_SEP}b`);
    expect(decodeCell(`a${ESC}Cb`)).toBe(`a${ESC}b`);
  });

  it("round-trips a value equal to the null marker", () => {
    expect(decodeCell(encodeCell(`${ESC}N`))).toBe(`${ESC}N`);
  });

  it("takes the fast path when no escape is present", () => {
    expect(decodeCell("plain value")).toBe("plain value");
  });
});

describe("decodePage", () => {
  it.each(fixtures)("matches the shared wire fixture: $name", ({ rows, packed }) => {
    expect(encodeRows(rows)).toBe(packed);
    expect(decodePage(packed)).toEqual(rows);
  });

  it("returns no rows for an empty payload", () => {
    expect(decodePage("")).toEqual([]);
  });

  it("round-trips mixed nulls, empty strings and text nulls", () => {
    roundTrip([
      ["1", null, "x"],
      [null, "", "null"],
      ["3", "y", null],
    ]);
  });

  it("round-trips consecutive escapes", () => {
    roundTrip([[`${ESC}${ESC}${CELL_SEP}${ROW_SEP}`]]);
  });

  it("round-trips multibyte content next to separators", () => {
    roundTrip([[`🦀${CELL_SEP}🦀${ROW_SEP}🦀`], ["árvíztűrő", "日本語"]]);
  });
});

describe("decodeColumns", () => {
  it("returns no columns for an empty header", () => {
    expect(decodeColumns("")).toEqual([]);
  });

  it("unescapes separators in column names", () => {
    expect(decodeColumns(`id${CELL_SEP}we${ESC}Aird`)).toEqual(["id", `we${CELL_SEP}ird`]);
  });
});

describe("decodeResult", () => {
  it("returns empty columns and rows for an empty payload", () => {
    expect(decodeResult("")).toEqual({ columns: [], rows: [] });
  });

  it("handles a header with no rows", () => {
    expect(decodeResult(`a${CELL_SEP}b`)).toEqual({ columns: ["a", "b"], rows: [] });
  });

  it("splits header from body on the first row separator only", () => {
    const packed = `a${CELL_SEP}b${ROW_SEP}1${CELL_SEP}2${ROW_SEP}3${CELL_SEP}4`;
    expect(decodeResult(packed)).toEqual({
      columns: ["a", "b"],
      rows: [
        ["1", "2"],
        ["3", "4"],
      ],
    });
  });

  it("keeps a NULL first cell of the first row out of the header", () => {
    const packed = `a${CELL_SEP}b${ROW_SEP}${ESC}N${CELL_SEP}2`;
    expect(decodeResult(packed)).toEqual({ columns: ["a", "b"], rows: [[null, "2"]] });
  });
});

describe("encodeResult", () => {
  it("preserves a single empty cell in a complete result", () => {
    const result = { columns: ["value"], rows: [[""]] };
    expect(decodeResult(encodeResult(result.columns, result.rows))).toEqual(result);
  });

  it("round-trips through decodeResult", () => {
    const columns = ["id", "name"];
    const rows: CellValue[][] = [
      ["1", null],
      [null, "null"],
      ["3", `has${CELL_SEP}sep`],
    ];
    expect(decodeResult(encodeResult(columns, rows))).toEqual({ columns, rows });
  });

  it("emits a header only when there are no rows", () => {
    expect(encodeResult(["a", "b"], [])).toBe(`a${CELL_SEP}b`);
  });

  it("keeps NULL and the empty string apart where a plain join would not", () => {
    expect(encodeRows([[null]])).not.toBe(encodeRows([[""]]));
  });
});
