import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { CellValue } from "@/lib/wire";

interface ResultsRecordProps {
  columns: string[];
  rows: CellValue[][];
}

export function ResultsRecord({ columns, rows }: ResultsRecordProps) {
  const [selection, setSelection] = useState({ rows, index: 0 });
  const selectedRow = selection.rows === rows ? selection.index : 0;
  const setSelectedRow = (index: number) => setSelection({ rows, index });

  if (rows.length === 0) {
    return (
      <div className="flex items-center justify-center p-4 text-muted-foreground">
        No rows to display
      </div>
    );
  }

  const safeIndex = Math.max(0, Math.min(selectedRow, rows.length - 1));

  return (
    <div className="flex-1 overflow-auto p-4">
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setSelectedRow(Math.max(0, safeIndex - 1))}
            disabled={safeIndex === 0}
          >
            Prev
          </Button>
          <span className="text-sm text-muted-foreground font-mono">
            Row {safeIndex + 1} of {rows.length.toLocaleString()}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setSelectedRow(Math.min(rows.length - 1, safeIndex + 1))}
            disabled={safeIndex >= rows.length - 1}
          >
            Next
          </Button>
        </div>
        <table className="w-full table-fixed border-collapse font-mono text-xs">
          <tbody>
            {columns.map((col, idx) => (
              <tr key={`${idx}:${col}`} className={idx % 2 === 1 ? "bg-muted/30" : ""}>
                <td className="w-1/3 break-words border-b border-border px-3 py-2 align-top font-semibold text-foreground">
                  {col}
                </td>
                <td className="whitespace-pre-wrap break-words border-b border-border px-3 py-2 text-foreground">
                  {rows[safeIndex]?.[idx] === null ? (
                    <span
                      className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground"
                      title="SQL NULL"
                    >
                      NULL
                    </span>
                  ) : rows[safeIndex]?.[idx] === "" ? (
                    <span className="italic text-muted-foreground">empty string</span>
                  ) : (
                    rows[safeIndex]?.[idx]
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
