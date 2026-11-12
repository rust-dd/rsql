import { CheckCircle2, Clock, ExternalLink, RotateCcw, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import type { StoredHistoryEntry } from "@/tauri";

interface HistoryRowProps {
  entry: StoredHistoryEntry;
  connection: string;
  onOpen: (entry: StoredHistoryEntry) => void;
  onReplace: (entry: StoredHistoryEntry) => void;
}

function formatWhen(timestamp: number): string {
  const time = new Date(timestamp);
  const sameDay = time.toDateString() === new Date().toDateString();
  return sameDay
    ? time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
    : time.toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}

export function HistoryRow({ entry, connection, onOpen, onReplace }: HistoryRowProps) {
  const preview = entry.sql.length > 240 ? `${entry.sql.slice(0, 240)}...` : entry.sql;
  const restorable = !entry.sqlTruncated;

  return (
    <div
      className={cn(
        "group flex items-start gap-3 border-b border-border px-4 py-2.5 transition-colors hover:bg-accent/50",
        restorable && "cursor-pointer",
      )}
      onClick={() => restorable && onOpen(entry)}
    >
      <div className="mt-0.5 flex-shrink-0">
        {entry.success ? (
          <CheckCircle2 className="h-3.5 w-3.5 text-success" />
        ) : (
          <XCircle className="h-3.5 w-3.5 text-destructive" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="line-clamp-4 whitespace-pre-wrap break-all font-mono text-xs leading-relaxed text-foreground">
          {preview}
        </div>
        {entry.error && (
          <div className="mt-1 truncate font-mono text-xs text-destructive">{entry.error}</div>
        )}
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <Clock className="h-2.5 w-2.5" />
            {formatWhen(entry.executedAt)}
          </span>
          <span className="truncate">
            {connection}
            {entry.database && ` · ${entry.database}`}
          </span>
          {entry.success && <span>{entry.rowCount.toLocaleString()} rows</span>}
          <span>{entry.durationMs.toFixed(0)}ms</span>
          {entry.sqlTruncated && (
            <span title="Only the beginning of this script was kept, so it cannot be restored">
              truncated
            </span>
          )}
        </div>
      </div>
      {restorable && (
        <div className="mt-0.5 flex flex-shrink-0 gap-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onOpen(entry);
            }}
            className="text-muted-foreground hover:text-foreground"
            title="Open in a new tab"
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onReplace(entry);
            }}
            className="text-muted-foreground hover:text-foreground"
            title="Replace the current editor"
          >
            <RotateCcw className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
    </div>
  );
}
