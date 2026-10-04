import {
  CheckCircle2,
  Clock,
  Diff,
  Edit3,
  GitBranch,
  History,
  Loader2,
  Pin,
  Search,
  Square,
  X,
  XCircle,
} from "lucide-react";
import { hasGeometryColumn } from "@/lib/geometry";
import { useUIStore } from "@/stores/ui-store";
import { ToolbarEdit } from "./toolbar-edit";
import { ToolbarExport } from "./toolbar-export";
import type { ToolbarProps } from "./types";
import { VirtualCsvExport } from "./virtual-csv-export";

export function ResultsToolbar(props: ToolbarProps) {
  const {
    panelView,
    setPanelView,
    result,
    columns,
    filteredRows,
    searchTerm,
    setSearchTerm,
    filteredCount,
    hasExplain,
    isExecuting,
    isEditing,
    editableTable,
    isCommitting,
    editError,
    pending,
    sessionMatchesEditor,
    confirmingApply,
    onEnterEdit,
    onRequestApply,
    onConfirmApply,
    onCancelApply,
    onDiscard,
    onCancel,
    virtualQuery,
  } = props;

  const pinnedResult = useUIStore((s) => s.pinnedResult);
  const pinResult = useUIStore((s) => s.pinResult);
  const clearPinnedResult = useUIStore((s) => s.clearPinnedResult);

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/50 bg-card/80 backdrop-blur px-4 py-2 flex-shrink-0">
      <div className="flex min-w-0 flex-wrap items-center gap-3">
        {/* Panel tabs — segment control */}
        <div className="inline-flex rounded-lg bg-muted p-0.5">
          <button
            type="button"
            aria-pressed={panelView === "grid"}
            onClick={() => {
              setPanelView("grid");
            }}
            className={`px-2 py-0.5 rounded-md text-xs font-mono transition-all duration-150 ${
              panelView === "grid"
                ? "bg-accent text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            Grid
          </button>
          <button
            type="button"
            aria-pressed={panelView === "record"}
            onClick={() => {
              setPanelView("record");
            }}
            className={`px-2 py-0.5 rounded-md text-xs font-mono transition-all duration-150 ${
              panelView === "record"
                ? "bg-accent text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
            disabled={!result?.rows.length || !!virtualQuery}
            title={virtualQuery ? "Narrow the query to use Record view." : undefined}
          >
            Record
          </button>
          {hasExplain && (
            <button
              type="button"
              aria-pressed={panelView === "explain"}
              onClick={() => setPanelView("explain")}
              className={`px-2 py-0.5 rounded-md text-xs font-mono transition-all duration-150 flex items-center gap-1 ${
                panelView === "explain"
                  ? "bg-accent text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              <GitBranch className="h-3 w-3" />
              Explain
            </button>
          )}
          <button
            type="button"
            aria-pressed={panelView === "history"}
            onClick={() => setPanelView("history")}
            className={`px-2 py-0.5 rounded-md text-xs font-mono transition-all duration-150 flex items-center gap-1 ${
              panelView === "history"
                ? "bg-accent text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            <History className="h-3 w-3" />
            History
          </button>
          {result && hasGeometryColumn(columns, filteredRows) && (
            <button
              type="button"
              aria-pressed={panelView === "map"}
              onClick={() => setPanelView("map")}
              className={`px-2 py-0.5 rounded-md text-xs font-mono transition-all duration-150 flex items-center gap-1 ${
                panelView === "map"
                  ? "bg-accent text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              Map
            </button>
          )}
        </div>

        {/* Result stats */}
        {panelView !== "history" && result && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            {isExecuting ? (
              <Loader2 className="h-3 w-3 animate-spin text-primary" />
            ) : result.status === "error" ? (
              <XCircle className="h-3 w-3 text-destructive" />
            ) : result.status === "cancelled" ? (
              <Square className="h-3 w-3" />
            ) : (
              <CheckCircle2 className="h-3 w-3 text-success" />
            )}
            <span>
              {result.status === "error"
                ? "Failed"
                : result.status === "cancelled"
                  ? "Cancelled"
                  : virtualQuery
                    ? `${virtualQuery.totalRows.toLocaleString()} rows`
                    : searchTerm
                      ? `${filteredCount.toLocaleString()} / ${result.rows.length.toLocaleString()} rows`
                      : `${result.rows.length.toLocaleString()} rows`}
              {result.capped && (
                <span
                  className="text-warning ml-1"
                  title="The result hit the in-memory limit. Rows beyond it were not kept — narrow the query to see them."
                >
                  (truncated)
                </span>
              )}
            </span>
            <span className="text-muted-foreground/50">&bull;</span>
            <Clock className="h-3 w-3" />
            <span>{result.time.toFixed(0)}ms</span>
            {isEditing && pending.updates ? (
              <>
                <span className="text-muted-foreground/50">&bull;</span>
                <span className="text-amber-500 font-medium">
                  {pending.updates} edit{pending.updates !== 1 ? "s" : ""}
                </span>
              </>
            ) : null}
            {isEditing && pending.deletes ? (
              <>
                <span className="text-muted-foreground/50">&bull;</span>
                <span className="text-destructive font-medium">
                  {pending.deletes} delete{pending.deletes !== 1 ? "s" : ""}
                </span>
              </>
            ) : null}
          </div>
        )}

        {/* Stop button — visible while executing */}
        {isExecuting && onCancel && (
          <button
            type="button"
            onClick={onCancel}
            className="flex items-center gap-1 px-2.5 py-1 rounded text-xs font-mono border border-destructive/50 text-destructive hover:bg-destructive/10 transition-colors"
          >
            <Square className="h-3 w-3" />
            Stop
          </button>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {/* Edit mode controls */}
        {isEditing ? (
          <ToolbarEdit
            pending={pending}
            sessionMatchesEditor={sessionMatchesEditor}
            confirmingApply={confirmingApply}
            editError={editError}
            isCommitting={isCommitting}
            onRequestApply={onRequestApply}
            onConfirmApply={onConfirmApply}
            onCancelApply={onCancelApply}
            onDiscard={onDiscard}
          />
        ) : (
          <>
            {/* Edit button */}
            {panelView !== "history" && editableTable && result && result.rows.length > 0 && (
              <button
                type="button"
                onClick={onEnterEdit}
                className="flex items-center gap-1 px-2 py-0.5 rounded text-xs font-mono text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
                title="Edit table data inline"
              >
                <Edit3 className="h-3 w-3" />
                Edit
              </button>
            )}

            {/* Pin / Diff */}
            {panelView !== "history" && result && result.rows.length > 0 && !virtualQuery && (
              <>
                {pinnedResult ? (
                  <div className="flex items-center gap-1 px-2 py-0.5 rounded text-xs font-mono bg-primary/10 text-primary border border-primary/20">
                    <Pin className="h-3 w-3" />
                    <span>Pinned: {pinnedResult.label}</span>
                    <button
                      type="button"
                      onClick={clearPinnedResult}
                      className="hover:text-destructive ml-1"
                      title="Clear pinned result"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() =>
                      pinResult(
                        { columns, rows: filteredRows, time: result.time },
                        `${filteredRows.length} rows`,
                      )
                    }
                    className="flex items-center gap-1 px-2 py-0.5 rounded text-xs font-mono text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
                    title="Pin current results for later diff comparison"
                  >
                    <Pin className="h-3 w-3" />
                    Pin
                  </button>
                )}
                {pinnedResult && (
                  <button
                    type="button"
                    onClick={() => setPanelView(panelView === "diff" ? "grid" : "diff")}
                    className={`flex items-center gap-1 px-2 py-0.5 rounded text-xs font-mono transition-colors ${
                      panelView === "diff"
                        ? "bg-primary text-primary-foreground"
                        : "text-muted-foreground hover:text-foreground hover:bg-accent"
                    }`}
                  >
                    <Diff className="h-3 w-3" />
                    Diff
                  </button>
                )}
              </>
            )}

            {/* Export dropdown */}
            {panelView !== "history" && result && virtualQuery && virtualQuery.totalRows > 0 && (
              <VirtualCsvExport
                key={virtualQuery.queryId}
                query={virtualQuery}
                columns={columns}
                capped={!!result.capped}
              />
            )}
            {panelView !== "history" && result && result.rows.length > 0 && !virtualQuery && (
              <ToolbarExport columns={columns} filteredRows={filteredRows} hasResult={!!result} />
            )}

            {/* Search */}
            {panelView !== "history" && result && columns.length > 0 && !virtualQuery && (
              <div className="relative flex items-center">
                <Search className="absolute left-2 h-3 w-3 text-muted-foreground pointer-events-none" />
                <input
                  type="text"
                  placeholder="Search results..."
                  aria-label="Search results"
                  onKeyDown={(event) => {
                    if (event.key === "Escape") setSearchTerm("");
                  }}
                  value={searchTerm}
                  onChange={(e) => setSearchTerm(e.target.value)}
                  className="h-7 w-48 rounded border border-border bg-input pl-7 pr-7 text-xs font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                />
                {searchTerm && (
                  <button
                    type="button"
                    onClick={() => setSearchTerm("")}
                    aria-label="Clear result search"
                    className="absolute right-2 text-muted-foreground hover:text-foreground"
                  >
                    <X className="h-3 w-3" />
                  </button>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
