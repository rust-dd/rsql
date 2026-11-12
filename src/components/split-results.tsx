import { ResultsGrid } from "@/components/results-grid";
import { QueryFeedback } from "@/components/results-panel/feedback";
import { useVirtualPaging } from "@/components/results-panel/use-virtual-paging";
import type { Tab } from "@/types";

interface SplitResultsProps {
  tab: Tab;
  onRetry: () => void;
}

export function SplitResults({ tab, onRetry }: SplitResultsProps) {
  const result = tab.splitResult;
  const vq = tab.splitVirtualQuery;
  const paging = useVirtualPaging({ vq, projectId: tab.projectId, slot: "split" });

  if (tab.isSplitExecuting) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <span className="animate-spin mr-2">⏳</span> Running...
      </div>
    );
  }
  if (result?.status && result.status !== "success") {
    return <QueryFeedback result={result} onRetry={onRetry} />;
  }
  if (!result) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground/40 text-sm font-mono">
        Run a query to see results
      </div>
    );
  }

  const rowCount = vq ? vq.totalRows : result.rows.length;
  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-1 border-b border-border/30 text-xs font-mono text-muted-foreground">
        <span>{rowCount.toLocaleString()} rows</span>
        {result.capped && (
          <span
            className="text-warning"
            title="The result hit the in-memory limit. Rows beyond it were not kept — narrow the query to see them."
          >
            (truncated)
          </span>
        )}
        {result.time > 0 && <span>· {result.time.toFixed(1)}ms</span>}
      </div>
      {paging.pageError && (
        <div className="flex items-center gap-2 border-b border-border/30 px-3 py-1 text-xs">
          <span className="min-w-0 flex-1 text-destructive">
            Could not load rows: {paging.pageError}
          </span>
          <button
            type="button"
            className="rounded border border-border px-2 py-0.5 hover:bg-accent"
            onClick={paging.retryPages}
          >
            Retry
          </button>
        </div>
      )}
      <div className="flex flex-1 min-h-0 flex-col">
        <ResultsGrid
          key={`${tab.id}:split`}
          columns={result.columns}
          rows={result.rows}
          virtualQuery={vq}
          onPageNeeded={vq ? paging.handlePageNeeded : undefined}
          onViewportRowChange={vq ? paging.handleViewportRowChange : undefined}
          restoreRowIndex={vq ? paging.restoreRowIndex : undefined}
          viewportKey={vq?.queryId}
          gridRef={paging.gridRef}
        />
      </div>
    </div>
  );
}
