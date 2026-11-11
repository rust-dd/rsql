import type { DataEditorProps } from "@glideapps/glide-data-grid";
import { renderToString } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { parseCellInput } from "@/lib/cell-input";
import { ResultsGrid } from ".";

const grid = vi.hoisted(() => ({ props: undefined as DataEditorProps | undefined }));

vi.mock("@glideapps/glide-data-grid", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@glideapps/glide-data-grid")>()),
  default: (props: DataEditorProps) => {
    grid.props = props;
    return null;
  },
}));

it.each([
  "NULL",
  "null",
  '"NULL"',
  '"123"',
  "",
  "  text  ",
])("preserves copied %j when pasted into a cell", (value) => {
  const edits: unknown[] = [];
  renderToString(
    <ResultsGrid
      columns={["value"]}
      rows={[[value], ["old"]]}
      isEditing
      onCellEdit={(_row, _column, input) => edits.push(parseCellInput(input))}
    />,
  );
  const props = grid.props;
  if (!props) throw new Error("Grid did not render");
  const source = props.getCellContent([0, 0]);
  const target = props.getCellContent([0, 1]);
  if (source.kind !== "text" || target.kind !== "text") throw new Error("Expected text cells");
  const copied = source.copyData ?? source.data;
  expect(copied).toBe(value);
  const pasted = props.coercePasteValue?.(copied, target) ?? { ...target, data: copied };
  if (pasted.kind !== "text") throw new Error("Expected a text paste");
  props.onCellEdited?.([0, 1], pasted);
  expect(edits).toEqual([value]);
});
