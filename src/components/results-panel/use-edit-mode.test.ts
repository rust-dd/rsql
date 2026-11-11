import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildCellContent,
  DELETED_OVERRIDE,
  FK_OVERRIDE,
} from "@/components/results-grid/rendering";
import { buildMutations, emptySession } from "@/lib/mutations";
import type { CellValue } from "@/lib/wire";
import type { Tab } from "@/types";
import { useEditMode } from "./use-edit-mode";

const { tabs, setCellEdit } = vi.hoisted(() => ({
  tabs: [] as Tab[],
  setCellEdit: vi.fn(),
}));

vi.mock("@/stores/tab-store", () => ({
  useTabStore: Object.assign((select: (state: { tabs: Tab[] }) => unknown) => select({ tabs }), {
    getState: () => ({ setCellEdit }),
  }),
}));

function editConsumer(original: CellValue = "42") {
  let editing: ReturnType<typeof useEditMode> | undefined;
  function Consumer() {
    editing = useEditMode({
      tabId: "edit-test",
      projectId: "project",
      editorValue: "SELECT * FROM public.items",
      result: { columns: ["id", "value"], rows: [["1", original]], time: 0 },
    });
    return null;
  }
  renderToString(createElement(Consumer));
  if (!editing) throw new Error("Edit consumer did not render");
  return editing;
}

beforeEach(() => {
  setCellEdit.mockReset();
  tabs.splice(0, tabs.length, {
    id: "edit-test",
    type: "query",
    title: "Items",
    editorValue: "SELECT * FROM public.items",
    isExecuting: false,
    editSession: emptySession("public", "items", ["id"]),
  });
});

describe("inline NULL edits", () => {
  it.each(["NULL", "null", "NuLl", "  NULL  "])("stages %s as SQL NULL", (input) => {
    editConsumer().handleCellEdit(0, 1, input);
    expect(setCellEdit).toHaveBeenCalledWith("edit-test", '[["id","1"]]', "value", null);
  });

  it.each([
    ["", ""],
    ['"NULL"', "NULL"],
    ['"null"', "null"],
    ['"123"', '"123"'],
    ['"quoted text"', '"quoted text"'],
    ['""', '""'],
    ["  text  ", "  text  "],
  ])("keeps %s distinct from SQL NULL", (input, expected) => {
    editConsumer().handleCellEdit(0, 1, input);
    expect(setCellEdit).toHaveBeenCalledWith("edit-test", '[["id","1"]]', "value", expected);
  });

  it("drops a pending edit when the original NULL is restored", () => {
    editConsumer(null).handleCellEdit(0, 1, "NULL");
    expect(setCellEdit).toHaveBeenCalledWith("edit-test", '[["id","1"]]', "value", undefined);
  });

  it("preserves pending NULL values for the grid and mutation payload", () => {
    const session = tabs[0].editSession;
    if (!session) throw new Error("Missing edit session");
    session.edits['[["id","1"]]'] = { value: null };
    expect(editConsumer().editedCells.get("0:1")).toBeNull();
    expect(buildMutations(session)).toEqual([
      { kind: "update", set: [["value", null]], pk: [["id", "1"]] },
    ]);
  });

  it.each([
    null,
    "",
    "NULL",
    "null",
    " NuLl ",
    '"NULL"',
    '"quoted text"',
    '""',
    '"escaped\\ntext"',
    '"unfinished',
    '{"value":null}',
    "O'Reilly",
    "árvíz 🦀\nsecond line",
    "42",
  ])("leaves %j unchanged when its editor is accepted without typing", (original) => {
    const cell = buildCellContent([1, 0], {
      rows: [["1", original]],
      isEditing: true,
      fkColIndices: new Set(),
      deletedOverride: DELETED_OVERRIDE,
      modifiedOverride: { bgCell: "orange" },
      fkOverride: FK_OVERRIDE,
    });
    if (cell.kind !== "text") throw new Error("Expected a text editor");
    editConsumer(original).handleCellEdit(0, 1, cell.data);
    expect(setCellEdit).toHaveBeenCalledWith("edit-test", '[["id","1"]]', "value", undefined);
  });
});
