import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import type { CellValue } from "@/lib/wire";

export type ExportFormat = "csv" | "json" | "sql" | "markdown" | "xml";

function escapeCSVText(value: string): string {
  if (value === "" || /[,"\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

/** SQL NULL becomes an unquoted empty field, the usual CSV convention. */
function escapeCSV(cell: CellValue): string {
  return cell === null ? "" : escapeCSVText(cell);
}

/**
 * Only a real SQL NULL becomes the NULL keyword. The text value "null" is
 * exported as a quoted literal, which earlier releases silently turned into NULL.
 */
function escapeSQL(cell: CellValue): string {
  if (cell === null) return "NULL";
  return `'${cell.replace(/'/g, "''")}'`;
}

function escapeXML(cell: CellValue): string {
  if (cell === null) return "";
  for (const character of cell) {
    const code = character.codePointAt(0) ?? 0;
    if (
      (code < 32 && code !== 9 && code !== 10 && code !== 13) ||
      (code >= 0xd800 && code <= 0xdfff) ||
      code === 0xfffe ||
      code === 0xffff
    ) {
      throw new Error(
        "XML cannot represent some control characters in this result. Use JSON or CSV.",
      );
    }
  }
  return cell
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/\r/g, "&#13;");
}

export function toCSV(columns: string[], rows: CellValue[][]): string {
  const header = columns.map(escapeCSVText).join(",");
  const body = rows.map((r) => r.map(escapeCSV).join(",")).join("\n");
  return `${header}\n${body}`;
}

/** SQL NULL is exported as JSON null rather than the string "null". */
export function toJSON(columns: string[], rows: CellValue[][]): string {
  if (new Set(columns).size !== columns.length) {
    throw new Error("JSON export requires unique column names. Add column aliases or use CSV.");
  }
  const objects = rows.map((row) =>
    Object.fromEntries(columns.map((column, i) => [column, row[i] ?? null])),
  );
  return JSON.stringify(objects, null, 2);
}

export function toSQL(columns: string[], rows: CellValue[][], tableName = "table_name"): string {
  if (rows.length === 0) return `-- No rows to export`;
  const quoteIdentifier = (value: string) => `"${value.replace(/"/g, '""')}"`;
  const colList = columns.map(quoteIdentifier).join(", ");
  return rows
    .map((row) => {
      const vals = row.map(escapeSQL).join(", ");
      return `INSERT INTO ${quoteIdentifier(tableName)} (${colList}) VALUES (${vals});`;
    })
    .join("\n");
}

export function toMarkdown(columns: string[], rows: CellValue[][]): string {
  const header = `| ${columns.join(" | ")} |`;
  const separator = `| ${columns.map(() => "---").join(" | ")} |`;
  const body = rows
    .map((r) => `| ${r.map((c) => (c === null ? "" : c.replace(/\|/g, "\\|"))).join(" | ")} |`)
    .join("\n");
  return `${header}\n${separator}\n${body}`;
}

export function toXML(columns: string[], rows: CellValue[][]): string {
  const lines = ['<?xml version="1.0" encoding="UTF-8"?>', "<resultset>"];
  for (const row of rows) {
    lines.push("  <row>");
    columns.forEach((col, i) => {
      const name = escapeXML(col).replace(/\n/g, "&#10;").replace(/\t/g, "&#9;");
      const value = row[i] ?? null;
      lines.push(
        value === null
          ? `    <column name="${name}" null="true" />`
          : `    <column name="${name}">${escapeXML(value)}</column>`,
      );
    });
    lines.push("  </row>");
  }
  lines.push("</resultset>");
  return lines.join("\n");
}

const formatters: Record<
  ExportFormat,
  (cols: string[], rows: CellValue[][], table?: string) => string
> = {
  csv: toCSV,
  json: toJSON,
  sql: toSQL,
  markdown: toMarkdown,
  xml: toXML,
};

const extensions: Record<ExportFormat, string> = {
  csv: "csv",
  json: "json",
  sql: "sql",
  markdown: "md",
  xml: "xml",
};

const filterNames: Record<ExportFormat, string> = {
  csv: "CSV Files",
  json: "JSON Files",
  sql: "SQL Files",
  markdown: "Markdown Files",
  xml: "XML Files",
};

export async function exportResults(
  format: ExportFormat,
  columns: string[],
  rows: CellValue[][],
  tableName?: string,
) {
  const content = formatters[format](columns, rows, tableName);
  const ext = extensions[format];

  const filePath = await save({
    defaultPath: `export.${ext}`,
    filters: [{ name: filterNames[format], extensions: [ext] }],
  });

  if (!filePath) return false;

  await writeTextFile(filePath, content);
  return true;
}

export function copyToClipboard(
  format: ExportFormat,
  columns: string[],
  rows: CellValue[][],
  tableName?: string,
): Promise<void> {
  const content = formatters[format](columns, rows, tableName);
  return navigator.clipboard.writeText(content);
}
