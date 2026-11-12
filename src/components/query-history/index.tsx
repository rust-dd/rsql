import { Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { EMPTY_HISTORY_FORM, toHistoryFilter } from "@/lib/history-filter";
import { useHistoryStore } from "@/stores/history-store";
import { projectName, useProjectStore } from "@/stores/project-store";
import { tabIdAt, useTabStore } from "@/stores/tab-store";
import { historyDatabases, historySearch, type StoredHistoryEntry } from "@/tauri";
import { Button } from "../ui/button";
import { HistoryRow } from "./history-row";
import { HistoryToolbar } from "./history-toolbar";

const PAGE_SIZE = 100;

function report(action: string) {
  return (error: unknown) => toast.error(`${action}: ${error}`);
}

export function QueryHistory() {
  const status = useHistoryStore((s) => s.status);
  const refreshStatus = useHistoryStore((s) => s.refreshStatus);
  const setEnabled = useHistoryStore((s) => s.setEnabled);
  const clearHistory = useHistoryStore((s) => s.clearHistory);
  const projects = useProjectStore((s) => s.projects);
  const openTab = useTabStore((s) => s.openTab);
  const updateContent = useTabStore((s) => s.updateContent);
  const selectedTabIndex = useTabStore((s) => s.selectedTabIndex);

  const [text, setText] = useState("");
  const [form, setForm] = useState(EMPTY_HISTORY_FORM);
  const [entries, setEntries] = useState<StoredHistoryEntry[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [databases, setDatabases] = useState<string[]>([]);
  const request = useRef(0);

  const connections = useMemo(
    () =>
      Object.keys(projects)
        .map((id): [string, string] => [id, projectName(projects, id)])
        .sort((a, b) => a[1].localeCompare(b[1])),
    [projects],
  );

  const load = useCallback(
    async (beforeId?: number) => {
      const id = ++request.current;
      setLoading(true);
      try {
        const filter = { ...toHistoryFilter(form, Date.now()), beforeId, limit: PAGE_SIZE };
        const page = await historySearch(filter);
        if (id !== request.current) return;
        setEntries((current) =>
          beforeId === undefined ? page.entries : [...current, ...page.entries],
        );
        setHasMore(page.hasMore);
      } catch (error) {
        if (id === request.current) report("Loading history failed")(error);
      } finally {
        if (id === request.current) setLoading(false);
      }
    },
    [form],
  );

  const loadDatabases = useCallback(() => {
    historyDatabases(form.projectId || undefined)
      .then(setDatabases)
      .catch(report("Loading history databases failed"));
  }, [form.projectId]);

  useEffect(() => {
    const id = window.setTimeout(() => setForm((f) => (f.text === text ? f : { ...f, text })), 200);
    return () => window.clearTimeout(id);
  }, [text]);

  useEffect(() => {
    refreshStatus().catch(report("Loading history settings failed"));
  }, [refreshStatus]);

  useEffect(() => {
    void load();
    loadDatabases();
    return useHistoryStore.subscribe((state, previous) => {
      if (state.revision === previous.revision) return;
      void load();
      loadDatabases();
    });
  }, [load, loadDatabases]);

  const openInNewTab = (entry: StoredHistoryEntry) =>
    openTab(projects[entry.projectId] ? entry.projectId : undefined, entry.sql);
  const replaceEditor = (entry: StoredHistoryEntry) => {
    const tabId = tabIdAt(selectedTabIndex);
    if (tabId) updateContent(tabId, entry.sql);
  };

  const filtered =
    form.text || form.projectId || form.database || form.status !== "all" || form.range !== "any";
  const empty = filtered ? "No queries match these filters" : "No saved queries yet";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <HistoryToolbar
        text={text}
        form={form}
        connections={connections}
        databases={databases}
        status={status}
        onText={setText}
        onForm={setForm}
        onEnabled={(enabled) =>
          setEnabled(enabled).catch(report("Changing history setting failed"))
        }
        onClear={() => clearHistory().catch(report("Clearing history failed"))}
      />
      {status && !status.enabled && (
        <div className="flex-shrink-0 border-b border-border bg-muted/40 px-4 py-1.5 text-xs text-muted-foreground">
          History recording is off. New queries are not saved.
        </div>
      )}
      <div className="flex-1 overflow-auto">
        {entries.map((entry) => (
          <HistoryRow
            key={entry.id}
            entry={entry}
            connection={
              projects[entry.projectId]
                ? projectName(projects, entry.projectId)
                : "Unknown connection"
            }
            onOpen={openInNewTab}
            onReplace={replaceEditor}
          />
        ))}
        {loading && (
          <div className="flex justify-center py-4 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
          </div>
        )}
        {!loading && entries.length === 0 && (
          <div className="py-10 text-center text-sm text-muted-foreground">{empty}</div>
        )}
        {!loading && hasMore && (
          <div className="flex justify-center py-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-7 text-xs"
              onClick={() => void load(entries[entries.length - 1]?.id)}
            >
              Load more
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
