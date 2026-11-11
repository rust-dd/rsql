import DataEditor, {
  CompactSelection,
  type DataEditorRef,
  type EditableGridCell,
  type GridCell,
  GridCellKind,
  type GridColumn,
  type GridSelection,
  type Item,
  type Theme,
} from "@glideapps/glide-data-grid";
import {
  type MutableRefObject,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import "@glideapps/glide-data-grid/dist/index.css";
import type { CellValue } from "@/lib/wire";
import { useUIStore } from "@/stores/ui-store";
import type { VirtualQuery } from "@/types";
import {
  buildCellContent,
  buildGridTheme,
  buildModifiedOverride,
  coercePasteValue,
  computeFkColIndices,
  computeGridColumns,
  DELETED_OVERRIDE,
  FK_OVERRIDE,
  GRID_ROW_HEIGHT,
} from "./rendering";

interface ResultsGridProps {
  columns: string[];
  rows: CellValue[][];
  isEditing?: boolean;
  cellEdits?: Map<string, CellValue>;
  deletedRows?: Set<number>;
  onCellEdit?: (rowIndex: number, colIndex: number, value: string) => void;
  onRowDelete?: (rowIndex: number) => void;
  onRowRestore?: (rowIndex: number) => void;
  fkColumns?: Map<string, { schema: string; table: string; column: string }>;
  onFKNavigate?: (colName: string, value: string) => void;
  virtualQuery?: VirtualQuery;
  onPageNeeded?: (pageIndex: number) => void;
  onViewportRowChange?: (topRow: number) => void;
  restoreRowIndex?: number; // can be fractional when smooth scroll is active
  viewportKey?: string;
  gridRef?: MutableRefObject<{ invalidatePage: (pageIndex: number) => void } | null>;
}

export function ResultsGrid({
  columns,
  rows,
  isEditing,
  cellEdits,
  deletedRows,
  onCellEdit,
  onRowDelete,
  onRowRestore,
  fkColumns,
  onFKNavigate,
  virtualQuery,
  onPageNeeded,
  onViewportRowChange,
  restoreRowIndex,
  viewportKey: _viewportKey,
  gridRef,
}: ResultsGridProps) {
  const theme = useUIStore((s) => s.theme);
  const containerRef = useRef<HTMLDivElement>(null);
  const dataEditorRef = useRef<DataEditorRef>(null);
  const [containerSize, setContainerSize] = useState({ width: 800, height: 400 });
  const visibleRangeRef = useRef({ y: 0, height: 0 });

  useImperativeHandle(
    gridRef ?? { current: null },
    () => ({
      invalidatePage: (_pageIndex: number) => {
        if (!virtualQuery) return;
        const totalRows = virtualQuery.totalRows;
        if (totalRows <= 0) return;

        const range = visibleRangeRef.current;
        const visibleStart = Math.max(0, Math.floor(range.y) - 2);
        const fallbackVisibleRows = Math.max(
          24,
          Math.ceil(containerSize.height / GRID_ROW_HEIGHT) + 4,
        );
        const effectiveHeight = range.height > 0 ? range.height : fallbackVisibleRows;
        const visibleEnd = Math.min(totalRows - 1, Math.ceil(range.y + effectiveHeight) + 2);
        if (visibleStart > visibleEnd) return;

        const cells: { cell: Item }[] = [];
        for (let row = visibleStart; row <= visibleEnd; row++) {
          for (let col = 0; col < columns.length; col++) {
            cells.push({ cell: [col, row] });
          }
        }
        if (cells.length > 0) {
          dataEditorRef.current?.updateCells(cells);
        }
      },
    }),
    [columns.length, containerSize.height, virtualQuery],
  );

  // Debounced via rAF to avoid mid-scroll re-renders
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    let rafId = 0;
    const obs = new ResizeObserver((entries) => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(() => {
        const { width, height } = entries[0].contentRect;
        const w = Math.round(width);
        const h = Math.round(height);
        setContainerSize((prev) =>
          prev.width === w && prev.height === h ? prev : { width: w, height: h },
        );
      });
    });
    obs.observe(el);
    return () => {
      cancelAnimationFrame(rafId);
      obs.disconnect();
    };
  }, []);

  const gridColumns = useMemo(
    (): GridColumn[] => computeGridColumns(columns, rows),
    [columns, rows],
  );

  const fkColIndices = useMemo(() => computeFkColIndices(columns, fkColumns), [columns, fkColumns]);

  // Pre-compute theme override objects — avoids creating new objects per cell render
  const deletedOverride = useMemo(() => DELETED_OVERRIDE, []);
  const modifiedOverride = useMemo(() => buildModifiedOverride(theme), [theme]);
  const fkOverride = useMemo(() => FK_OVERRIDE, []);

  const totalRowCount = virtualQuery ? virtualQuery.totalRows : rows.length;

  // Restore previous viewport row when switching back to a tab/query
  useLayoutEffect(() => {
    if (typeof restoreRowIndex !== "number" || totalRowCount <= 0) return;
    const targetRow = Math.min(restoreRowIndex, totalRowCount - 1);
    if (targetRow > 0) {
      dataEditorRef.current?.scrollTo(0, { amount: targetRow, unit: "cell" }, "vertical", 0, 0, {
        vAlign: "start",
      });
    }
    visibleRangeRef.current = { ...visibleRangeRef.current, y: targetRow };
    onViewportRowChange?.(targetRow);
  }, [restoreRowIndex, totalRowCount, onViewportRowChange]);

  const getCellContent = useCallback(
    (cell: Item): GridCell =>
      buildCellContent(cell, {
        rows,
        cellEdits,
        deletedRows,
        isEditing,
        fkColIndices,
        virtualQuery,
        onPageNeeded,
        deletedOverride,
        modifiedOverride,
        fkOverride,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      rows,
      cellEdits,
      deletedRows,
      isEditing,
      fkColIndices,
      virtualQuery,
      onPageNeeded,
      fkOverride,
      modifiedOverride,
      deletedOverride,
    ],
  );

  // Trigger page loads on scroll, throttled via rAF
  const scrollRafId = useRef(0);
  const handleVisibleRegionChanged = useCallback(
    (range: { x: number; y: number; width: number; height: number }) => {
      visibleRangeRef.current = { y: range.y, height: range.height };
      onViewportRowChange?.(Math.max(0, range.y));
      if (!virtualQuery || !onPageNeeded) return;
      cancelAnimationFrame(scrollRafId.current);
      scrollRafId.current = requestAnimationFrame(() => {
        const { y, height } = range;
        const ps = virtualQuery.pageSize;
        const firstVisible = Math.floor(y / ps);
        const lastVisible = Math.floor((y + height) / ps);
        for (let p = firstVisible - 1; p <= lastVisible + 3; p++) {
          if (p >= 0 && p * ps < virtualQuery.totalRows) {
            onPageNeeded(p);
          }
        }
      });
    },
    [virtualQuery, onPageNeeded, onViewportRowChange],
  );

  useEffect(
    () => () => {
      cancelAnimationFrame(scrollRafId.current);
    },
    [],
  );

  const onCellEdited = useCallback(
    (cell: Item, newVal: EditableGridCell) => {
      if (newVal.kind !== GridCellKind.Text) return;
      const [colIdx, rowIdx] = cell;
      onCellEdit?.(rowIdx, colIdx, newVal.data);
    },
    [onCellEdit],
  );

  const handleRowClick = useCallback(
    (cell: Item) => {
      if (isEditing || virtualQuery) return;
      const [colIdx, rowIdx] = cell;

      if (fkColIndices.has(colIdx) && onFKNavigate) {
        const colName = columns[colIdx];
        const value = rows[rowIdx]?.[colIdx];
        if (value) {
          onFKNavigate(colName, value);
        }
      }
    },
    [isEditing, virtualQuery, fkColIndices, onFKNavigate, columns, rows],
  );

  const gridTheme = useMemo((): Partial<Theme> => buildGridTheme(theme), [theme]);

  const rowMarkers = isEditing ? ("checkbox-visible" as const) : ("none" as const);
  const [selection, setSelection] = useState<GridSelection>({
    rows: CompactSelection.empty(),
    columns: CompactSelection.empty(),
  });

  useEffect(() => {
    if (!isEditing) {
      setSelection({ rows: CompactSelection.empty(), columns: CompactSelection.empty() });
    }
  }, [isEditing]);

  useEffect(() => {
    if (!isEditing || !deletedRows) return;
    let sel = CompactSelection.empty();
    for (const idx of deletedRows) {
      sel = sel.add(idx);
    }
    setSelection((prev) => ({ ...prev, rows: sel }));
  }, [isEditing, deletedRows]);

  const selectionRef = useRef(selection);
  selectionRef.current = selection;

  const handleSelectionChange = useCallback(
    (newSel: GridSelection) => {
      if (!isEditing) return;
      const oldRows = selectionRef.current.rows;
      const newRows = newSel.rows;

      for (let i = 0; i < rows.length; i++) {
        const wasSelected = oldRows.hasIndex(i);
        const isSelected = newRows.hasIndex(i);
        if (isSelected && !wasSelected) onRowDelete?.(i);
        if (!isSelected && wasSelected) onRowRestore?.(i);
      }

      setSelection(newSel);
    },
    [isEditing, rows.length, onRowDelete, onRowRestore],
  );

  return (
    <div ref={containerRef} className="results-grid-scroll flex-1 min-h-0 overflow-hidden">
      <DataEditor
        ref={dataEditorRef}
        columns={gridColumns}
        rows={totalRowCount}
        getCellContent={getCellContent}
        onCellEdited={isEditing ? onCellEdited : undefined}
        coercePasteValue={coercePasteValue}
        onCellClicked={handleRowClick}
        onVisibleRegionChanged={handleVisibleRegionChanged}
        theme={gridTheme}
        width={containerSize.width}
        height={containerSize.height}
        smoothScrollX
        smoothScrollY={!virtualQuery}
        experimental={{ renderStrategy: "direct" }}
        rowMarkers={rowMarkers}
        gridSelection={isEditing ? selection : undefined}
        onGridSelectionChange={isEditing ? handleSelectionChange : undefined}
        getCellsForSelection={true}
        keybindings={{ search: !virtualQuery }}
        overscrollX={0}
        overscrollY={0}
        rowHeight={GRID_ROW_HEIGHT}
        headerHeight={34}
      />
    </div>
  );
}
