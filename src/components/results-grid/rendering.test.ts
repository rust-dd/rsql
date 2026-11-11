import { GridCellKind } from "@glideapps/glide-data-grid";
import { describe, expect, it } from "vitest";
import {
  buildCellContent,
  type CellContentContext,
  DELETED_OVERRIDE,
  FK_OVERRIDE,
} from "./rendering";

const context: CellContentContext = {
  rows: [[null, "NULL", ""]],
  isEditing: true,
  fkColIndices: new Set(),
  deletedOverride: DELETED_OVERRIDE,
  modifiedOverride: { bgCell: "orange" },
  fkOverride: FK_OVERRIDE,
};

describe("NULL cells in edit mode", () => {
  it("opens an existing SQL NULL as NULL instead of an empty string", () => {
    const cell = buildCellContent([0, 0], context);
    expect(cell).toMatchObject({ kind: GridCellKind.Text, data: "NULL", displayData: "NULL" });
  });

  it("quotes literal NULL text in the editor without changing its display", () => {
    const cell = buildCellContent([1, 0], context);
    expect(cell).toMatchObject({ data: '"NULL"', displayData: "NULL" });
  });

  it("leaves empty strings empty", () => {
    expect(buildCellContent([2, 0], context)).toMatchObject({ data: "", displayData: "" });
  });

  it("keeps read-only text unchanged", () => {
    expect(buildCellContent([1, 0], { ...context, isEditing: false })).toMatchObject({
      data: "NULL",
      displayData: "NULL",
    });
  });

  it("displays a pending NULL assignment and keeps it NULL when reopened", () => {
    expect(
      buildCellContent([1, 0], { ...context, cellEdits: new Map([["0:1", null]]) }),
    ).toMatchObject({ data: "NULL", displayData: "NULL", themeOverride: { bgCell: "orange" } });
  });

  it("copies literal text without editor quoting", () => {
    expect(buildCellContent([1, 0], context)).toMatchObject({ copyData: "NULL" });
  });
});
