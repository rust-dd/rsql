/**
 * Packed wire format shared by every query path. Mirrors
 * `src-tauri/src/drivers/pgsql/wire.rs` — the two must be changed together.
 *
 * Cells are joined by CELL_SEP, rows by ROW_SEP. Because any byte can appear in
 * a Postgres text value, separators occurring inside data are escaped with ESC
 * rather than replaced. SQL NULL has its own encoding so it stays distinct from
 * the text value "null" and from the empty string. Empty strings have their
 * own marker so a single empty cell cannot be mistaken for an empty page.
 */

export const CELL_SEP = "\x1F";
export const ROW_SEP = "\x1E";
export const ESC = "\x1D";

const NULL_MARKER = `${ESC}N`;
const EMPTY_MARKER = `${ESC}E`;
const TAG_CELL_SEP = "A";
const TAG_ROW_SEP = "B";
const TAG_ESC = "C";

/** A single cell: `null` is SQL NULL, `""` is the empty string. */
export type CellValue = string | null;

/**
 * Encode one cell. Mirrors `push_cell` in wire.rs. Needed wherever the frontend
 * sends rows back to Rust — a plain `join` would turn NULL into "" and would
 * leave separators inside data unescaped.
 */
export function encodeCell(cell: CellValue): string {
  if (cell === null) return NULL_MARKER;
  if (cell === "") return EMPTY_MARKER;
  if (!cell.includes(CELL_SEP) && !cell.includes(ROW_SEP) && !cell.includes(ESC)) {
    return cell;
  }

  let out = "";
  for (const ch of cell) {
    if (ch === CELL_SEP) out += `${ESC}${TAG_CELL_SEP}`;
    else if (ch === ROW_SEP) out += `${ESC}${TAG_ROW_SEP}`;
    else if (ch === ESC) out += `${ESC}${TAG_ESC}`;
    else out += ch;
  }
  return out;
}

/** Encode one row. */
export function encodeRow(row: CellValue[]): string {
  return row.map(encodeCell).join(CELL_SEP);
}

/** Encode rows into a packed page body (no header). */
export function encodeRows(rows: CellValue[][]): string {
  return rows.map(encodeRow).join(ROW_SEP);
}

/** Encode a full packed result: header line followed by row lines. */
export function encodeResult(columns: string[], rows: CellValue[][]): string {
  const header = encodeRow(columns);
  if (rows.length === 0) return header;
  return `${header}${ROW_SEP}${encodeRows(rows)}`;
}

/** Decode one cell, resolving the NULL marker and unescaping separators. */
export function decodeCell(raw: string): CellValue {
  if (raw === NULL_MARKER) return null;
  if (raw === EMPTY_MARKER) return "";
  if (!raw.includes(ESC)) return raw;

  let out = "";
  for (let i = 0; i < raw.length; i++) {
    if (raw[i] !== ESC) {
      out += raw[i];
      continue;
    }
    i += 1;
    switch (raw[i]) {
      case TAG_CELL_SEP:
        out += CELL_SEP;
        break;
      case TAG_ROW_SEP:
        out += ROW_SEP;
        break;
      case TAG_ESC:
        out += ESC;
        break;
      default:
        if (raw[i] !== undefined) out += raw[i];
        break;
    }
  }
  return out;
}

/** Decode one packed row. */
export function decodeRow(raw: string): CellValue[] {
  return raw.split(CELL_SEP).map(decodeCell);
}

/** Decode a packed page body (rows only, no header). */
export function decodePage(packed: string): CellValue[][] {
  if (!packed) return [];
  return packed.split(ROW_SEP).map(decodeRow);
}

/** Decode a packed header line into column names. */
export function decodeColumns(raw: string): string[] {
  if (!raw) return [];
  return raw.split(CELL_SEP).map((cell) => decodeCell(cell) ?? "");
}

/** Decode a full packed result: header line followed by row lines. */
export function decodeResult(packed: string): { columns: string[]; rows: CellValue[][] } {
  if (!packed) return { columns: [], rows: [] };
  const separator = packed.indexOf(ROW_SEP);
  if (separator === -1) {
    return { columns: decodeColumns(packed), rows: [] };
  }
  return {
    columns: decodeColumns(packed.slice(0, separator)),
    rows: decodePage(packed.slice(separator + 1)),
  };
}

/**
 * Render a cell for display and for text-oriented operations such as search,
 * export and diffing. NULL renders as the empty string so it never masquerades
 * as the text "null"; callers that must show NULL distinctly check for `null`
 * themselves before calling this.
 */
export function cellText(cell: CellValue): string {
  return cell ?? "";
}
