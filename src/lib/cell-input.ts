import type { CellValue } from "./wire";

const NULL_INPUT = /^("*)(\s*null\s*)\1$/i;

/** Bare NULL is SQL NULL; one outer quote pair escapes a literal NULL token. */
export function parseCellInput(input: string): CellValue {
  const match = NULL_INPUT.exec(input);
  if (!match) return input;
  return match[1].length === 0 ? null : input.slice(1, -1);
}

/** Opening and accepting a cell must preserve its value, including literal NULL text. */
export function formatCellInput(value: CellValue): string {
  if (value === null) return "NULL";
  return NULL_INPUT.test(value) ? `"${value}"` : value;
}
