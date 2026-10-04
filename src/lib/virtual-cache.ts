import type { CellValue } from "@/lib/wire";

// Virtual pagination page cache
// Each query gets a Map of pageIndex → rows (CellValue[][])
// Max pages per query controlled by evictDistant()

const cache = new Map<string, Map<number, CellValue[][]>>();
const viewportRows = new Map<string, number>();

export function getViewportRow(queryId: string): number {
  return viewportRows.get(queryId) ?? 0;
}

export function setViewportRow(queryId: string, row: number): void {
  viewportRows.set(queryId, row);
}

export function setPage(queryId: string, pageIndex: number, rows: CellValue[][]): void {
  if (!cache.has(queryId)) cache.set(queryId, new Map());
  cache.get(queryId)?.set(pageIndex, rows);
}

export function getRow(queryId: string, rowIndex: number, pageSize: number): CellValue[] | null {
  const pages = cache.get(queryId);
  if (!pages) return null;
  const pageIndex = Math.floor(rowIndex / pageSize);
  const page = pages.get(pageIndex);
  if (!page) return null;
  const localIndex = rowIndex - pageIndex * pageSize;
  return page[localIndex] ?? null;
}

export function hasPage(queryId: string, pageIndex: number): boolean {
  return cache.get(queryId)?.has(pageIndex) ?? false;
}

export function clearQuery(queryId: string): void {
  cache.delete(queryId);
  viewportRows.delete(queryId);
}

export function evictDistant(queryId: string, currentPage: number, maxPages: number): void {
  const pages = cache.get(queryId);
  if (!pages || pages.size <= maxPages) return;
  const indices = [...pages.keys()].sort(
    (a, b) => Math.abs(a - currentPage) - Math.abs(b - currentPage),
  );
  const toKeep = new Set(indices.slice(0, maxPages));
  for (const key of pages.keys()) {
    if (!toKeep.has(key)) pages.delete(key);
  }
}
