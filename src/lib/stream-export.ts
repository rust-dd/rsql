import { csvRows, toCSV } from "./export";
import { decodePage } from "./wire";

export async function streamVirtualCSV({
  columns,
  totalRows,
  pageSize,
  fetchPage,
  write,
  signal,
  onProgress,
}: {
  columns: string[];
  totalRows: number;
  pageSize: number;
  fetchPage(offset: number, limit: number): Promise<string>;
  write(bytes: Uint8Array): Promise<number>;
  signal: AbortSignal;
  onProgress(rows: number): void;
}) {
  if (pageSize <= 0) throw new Error("Invalid result page size");
  const encoder = new TextEncoder();
  const append = async (text: string) => {
    const bytes = encoder.encode(text);
    let offset = 0;
    while (offset < bytes.length) {
      signal.throwIfAborted();
      const written = await write(bytes.subarray(offset));
      if (written <= 0 || written > bytes.length - offset)
        throw new Error("Could not write the CSV file");
      offset += written;
    }
  };
  await append(toCSV(columns, []));
  for (let offset = 0; offset < totalRows; offset += pageSize) {
    signal.throwIfAborted();
    const rows = decodePage(await fetchPage(offset, pageSize));
    if (
      rows.length !== Math.min(pageSize, totalRows - offset) ||
      rows.some((row) => row.length !== columns.length)
    ) {
      throw new Error(
        "This result is incomplete or has expired. Run the query again before exporting.",
      );
    }
    await append(`${csvRows(rows)}\n`);
    onProgress(offset + rows.length);
  }
  signal.throwIfAborted();
}
