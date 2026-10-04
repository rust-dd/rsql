import { Loader2, X, XCircle } from "lucide-react";
import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { hasGeometryColumn } from "@/lib/geometry";
import { cancelTabQuery, executeTabQuery } from "@/lib/query-execution";
import { cellText } from "@/lib/wire";
import { useActiveTab } from "@/stores/tab-store";
import { useUIStore } from "@/stores/ui-store";
import { ResultsGrid } from "../results-grid";
import { ResultsRecord } from "../results-record";
import { EmptyResults, QueryFeedback } from "./feedback";
import { ResultsToolbar } from "./toolbar";
import type { PanelView } from "./types";
import { useEditMode } from "./use-edit-mode";
import { useVirtualPaging } from "./use-virtual-paging";

const ExplainPanel = lazy(() =>
  import("../explain-panel").then((module) => ({ default: module.ExplainPanel })),
);
const QueryHistory = lazy(() =>
  import("../query-history").then((module) => ({ default: module.QueryHistory })),
);
const ResultsMap = lazy(() =>
  import("../results-map").then((module) => ({ default: module.ResultsMap })),
);
const DiffView = lazy(() => import("./diff-view").then((module) => ({ default: module.DiffView })));

export function ResultsPanel() {
  const activeTab = useActiveTab();
  return (
    <Suspense
      fallback={
        <div role="status" className="p-4 text-sm text-muted-foreground">
          Loading view…
        </div>
      }
    >
      <ResultsPanelContent key={activeTab?.id ?? "empty"} />
    </Suspense>
  );
}

function ResultsPanelContent() {
  const activeTab = useActiveTab();
  const pinnedResult = useUIStore((s) => s.pinnedResult);
  const [requestedView, setPanelView] = useState<PanelView>("grid");
  const result = activeTab?.result;
  const [filter, setFilter] = useState({ result, term: "" });
  const [debouncedFilter, setDebouncedFilter] = useState(filter);
  const searchTerm = filter.result === result ? filter.term : "";
  const debouncedSearch = debouncedFilter.result === result ? debouncedFilter.term : "";
  const setSearchTerm = useCallback((term: string) => setFilter({ result, term }), [result]);

  useEffect(() => {
    if (!filter.term.trim()) {
      setDebouncedFilter(filter);
      return;
    }
    const timer = setTimeout(() => setDebouncedFilter(filter), 200);
    return () => clearTimeout(timer);
  }, [filter]);

  const isExecuting = activeTab?.isExecuting;
  const vq = activeTab?.virtualQuery;

  const handleCancel = useCallback(() => cancelTabQuery(), []);
  const panelView =
    (requestedView === "record" && (vq || !result?.rows.length)) ||
    (requestedView === "explain" && !activeTab?.explainResult) ||
    (requestedView === "diff" && !pinnedResult) ||
    (requestedView === "map" && (!result || !hasGeometryColumn(result.columns, result.rows))) ||
    (requestedView !== "history" && result?.status && result.status !== "success")
      ? "grid"
      : requestedView;

  const {
    gridRef,
    handlePageNeeded,
    handleViewportRowChange,
    restoreRowIndex,
    pageError,
    retryPages,
  } = useVirtualPaging({
    vq,
    projectId: activeTab?.projectId,
  });

  const {
    isEditing,
    editError,
    setEditError,
    isCommitting,
    confirmingApply,
    pending,
    sessionMatchesEditor,
    editableTable,
    fkMap,
    editedCells,
    deletedRowIndices,
    handleFKNavigate,
    handleEnterEdit,
    handleDiscard,
    handleRequestApply,
    handleConfirmApply,
    handleCancelApply,
    handleCellEdit,
    handleRowDelete,
    handleRowRestore,
  } = useEditMode({
    tabId: activeTab?.id,
    projectId: activeTab?.projectId,
    editorValue: activeTab?.editorValue,
    result,
  });

  const filteredRows = useMemo(() => {
    if (isEditing || vq) return result?.rows ?? [];
    if (!result || !debouncedSearch.trim()) return result?.rows ?? [];
    const term = debouncedSearch.toLowerCase();
    return result.rows.filter((row) =>
      row.some((cell) => cellText(cell).toLowerCase().includes(term)),
    );
  }, [result, debouncedSearch, isEditing, vq]);

  const explainResult = activeTab?.explainResult;
  const hasExplain = !!explainResult;

  const toolbarProps = {
    panelView,
    setPanelView,
    searchTerm,
    setSearchTerm,
    hasExplain,
    isExecuting: !!isExecuting,
    isEditing,
    editableTable: !!editableTable && !vq,
    isCommitting,
    editError,
    pending,
    sessionMatchesEditor,
    confirmingApply,
    onEnterEdit: handleEnterEdit,
    onRequestApply: handleRequestApply,
    onConfirmApply: handleConfirmApply,
    onCancelApply: handleCancelApply,
    onDiscard: handleDiscard,
    onCancel: handleCancel,
    virtualQuery: vq,
  };

  if (panelView === "explain" && hasExplain) {
    return (
      <div className="flex h-full flex-col border-t border-border bg-card">
        <ResultsToolbar
          {...toolbarProps}
          result={result ?? null}
          columns={result?.columns ?? []}
          filteredRows={filteredRows}
          filteredCount={filteredRows.length}
        />
        <ExplainPanel plan={explainResult} />
      </div>
    );
  }

  if (panelView !== "history" && isExecuting && !result) {
    return (
      <div className="flex h-full flex-col">
        <ResultsToolbar
          {...toolbarProps}
          result={null}
          columns={[]}
          filteredRows={[]}
          filteredCount={0}
        />
        <div className="flex flex-1 items-center justify-center text-muted-foreground gap-2">
          <Loader2 className="h-5 w-5 animate-spin" />
          <span className="text-sm">Executing query...</span>
        </div>
      </div>
    );
  }

  if (panelView === "history") {
    return (
      <div className="flex h-full flex-col border-t border-border bg-card">
        <ResultsToolbar
          {...toolbarProps}
          result={result ?? null}
          columns={result?.columns ?? []}
          filteredRows={filteredRows}
          filteredCount={filteredRows.length}
        />
        <QueryHistory />
      </div>
    );
  }

  if (panelView === "diff" && pinnedResult && result) {
    return (
      <div className="flex h-full flex-col border-t border-border bg-card">
        <ResultsToolbar
          {...toolbarProps}
          result={result}
          columns={result.columns}
          filteredRows={filteredRows}
          filteredCount={filteredRows.length}
        />
        <DiffView
          pinnedColumns={pinnedResult.columns}
          pinnedRows={pinnedResult.rows}
          currentColumns={result.columns}
          currentRows={filteredRows}
        />
      </div>
    );
  }

  if (result?.status && result.status !== "success") {
    return (
      <div className="flex h-full min-h-0 flex-col border-t border-border bg-card">
        <ResultsToolbar
          {...toolbarProps}
          result={result}
          columns={[]}
          filteredRows={[]}
          filteredCount={0}
        />
        <QueryFeedback result={result} onRetry={() => void executeTabQuery()} />
      </div>
    );
  }

  if (panelView === "map" && result) {
    return (
      <div className="flex h-full flex-col border-t border-border bg-card">
        <ResultsToolbar
          {...toolbarProps}
          result={result}
          columns={result.columns}
          filteredRows={filteredRows}
          filteredCount={filteredRows.length}
        />
        {vq && (
          <p
            role="status"
            className="border-b border-border px-4 py-2 text-xs text-muted-foreground"
          >
            Map shows the first {result.rows.length.toLocaleString()} of{" "}
            {vq.totalRows.toLocaleString()} rows. Narrow your query to map the full result.
          </p>
        )}
        <ResultsMap columns={result.columns} rows={filteredRows} />
      </div>
    );
  }

  if (!result) {
    return (
      <div className="flex h-full flex-col">
        <ResultsToolbar
          {...toolbarProps}
          result={null}
          columns={[]}
          filteredRows={[]}
          filteredCount={0}
        />
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <div className="space-y-2 text-center">
            <p className="text-sm font-medium text-foreground">Run a query to see results</p>
            <p className="text-xs">Choose a connection, write SQL, then select Execute.</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col border-t border-border bg-card">
      <ResultsToolbar
        {...toolbarProps}
        result={result}
        columns={result.columns}
        filteredRows={filteredRows}
        filteredCount={filteredRows.length}
      />
      {editError && !isEditing && (
        <div className="flex items-center gap-2 px-4 py-1.5 bg-destructive/10 text-destructive text-xs border-b border-border">
          <XCircle className="h-3 w-3" />
          {editError}
          <button
            type="button"
            onClick={() => setEditError(null)}
            className="ml-auto hover:text-foreground"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      )}
      {pageError && (
        <div
          role="alert"
          className="flex items-center gap-3 border-b border-border bg-destructive/5 px-4 py-2 text-xs"
        >
          <span className="min-w-0 flex-1 text-destructive">Could not load rows: {pageError}</span>
          <button
            type="button"
            onClick={retryPages}
            className="shrink-0 rounded border border-border px-2 py-1 hover:bg-accent"
          >
            Retry
          </button>
        </div>
      )}
      {panelView !== "record" ? (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <ResultsGrid
            key={activeTab?.id ?? "results-grid"}
            columns={result.columns}
            rows={filteredRows}
            isEditing={isEditing}
            cellEdits={editedCells}
            deletedRows={deletedRowIndices}
            onCellEdit={handleCellEdit}
            onRowDelete={handleRowDelete}
            onRowRestore={handleRowRestore}
            fkColumns={fkMap}
            onFKNavigate={handleFKNavigate}
            virtualQuery={vq}
            onPageNeeded={vq ? handlePageNeeded : undefined}
            onViewportRowChange={vq ? handleViewportRowChange : undefined}
            restoreRowIndex={vq ? restoreRowIndex : undefined}
            viewportKey={vq?.queryId}
            gridRef={gridRef}
          />
          {!vq && filteredRows.length === 0 && !isExecuting && result.columns.length > 0 && (
            <EmptyResults filtered={!!debouncedSearch.trim()} onClear={() => setSearchTerm("")} />
          )}
        </div>
      ) : (
        <ResultsRecord columns={result.columns} rows={filteredRows} />
      )}
    </div>
  );
}
