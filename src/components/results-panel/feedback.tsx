import { Database, Search, Square, XCircle } from "lucide-react";
import type { QueryResult } from "@/types";

export function QueryFeedback({ result, onRetry }: { result: QueryResult; onRetry?: () => void }) {
  const failed = result.status === "error";
  const Icon = failed ? XCircle : Square;
  return (
    <div
      className="flex min-h-0 flex-1 items-start gap-3 overflow-auto p-5"
      role={failed ? "alert" : "status"}
    >
      <Icon
        className={`mt-0.5 h-5 w-5 shrink-0 ${failed ? "text-destructive" : "text-muted-foreground"}`}
      />
      <div className="min-w-0 space-y-2">
        <p className="text-sm font-medium">{failed ? "Query failed" : "Query cancelled"}</p>
        <pre className="whitespace-pre-wrap break-words font-mono text-xs text-muted-foreground">
          {result.message}
        </pre>
        {onRetry && (
          <button
            type="button"
            onClick={onRetry}
            className="rounded border border-border px-3 py-1.5 text-xs hover:bg-accent"
          >
            Run again
          </button>
        )}
      </div>
    </div>
  );
}

export function EmptyResults({ filtered, onClear }: { filtered: boolean; onClear: () => void }) {
  const Icon = filtered ? Search : Database;
  return (
    <div
      className="absolute inset-x-0 bottom-0 top-10 flex flex-col items-center justify-center gap-2 p-4 text-center"
      role="status"
    >
      <Icon className="h-5 w-5 text-muted-foreground/60" />
      <p className="text-sm font-medium">{filtered ? "No matching rows" : "No rows returned"}</p>
      <p className="text-xs text-muted-foreground">
        {filtered
          ? "Try a different search or clear the filter."
          : "The query completed successfully."}
      </p>
      {filtered && (
        <button
          type="button"
          onClick={onClear}
          className="rounded border border-border px-3 py-1 text-xs hover:bg-accent"
        >
          Clear filter
        </button>
      )}
    </div>
  );
}
