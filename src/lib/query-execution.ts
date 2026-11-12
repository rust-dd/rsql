import { toast } from "sonner";
import { DriverFactory } from "@/lib/database-driver";
import { changesSchema } from "@/lib/ddl-detect";
import { editorSelection } from "@/lib/editor-selection";
import { isQueryCancelledError, notifyQueryComplete, PAGE_SIZE } from "@/lib/query-helpers";
import { runTarget, targetStatement } from "@/lib/sql-statements";
import {
  cancelExecution,
  releaseResult,
  type Slot,
  trackExecution,
  trackResult,
} from "@/lib/tab-resources";
import * as virtualCache from "@/lib/virtual-cache";
import { decodeColumns, decodePage, decodeResult } from "@/lib/wire";
import { useHistoryStore } from "@/stores/history-store";
import { useProjectStore } from "@/stores/project-store";
import { useSchemaIndexStore } from "@/stores/schema-index-store";
import { useTabStore } from "@/stores/tab-store";
import type { ExplainPlan, QueryResult } from "@/types";

export type ExecutionMode = "query" | "explain" | "analyze" | "split";
/** A statement run covers the pane's selection or caret statement; a script run covers the pane. */
export type ExecutionScope = "statement" | "script";

export function executionError(error: unknown, elapsed: number): QueryResult {
  const message = error instanceof Error ? error.message : String(error);
  return {
    columns: [],
    rows: [],
    time: elapsed,
    status: isQueryCancelledError(message) ? "cancelled" : "error",
    message,
  };
}

/** Explain targets one statement of the main editor; an ambiguous target runs nothing. */
function explainTarget(tabId: string, sql: string): string | undefined {
  const editor = editorSelection(tabId, "main");
  const target = targetStatement(editor?.text ?? sql, editor);
  if ("statement" in target) return target.statement.text;
  toast.error(target.error);
  return undefined;
}

/** The selected SQL, or else the statement at the caret; an ambiguous caret runs nothing. */
function runTargetSql(tabId: string, slot: Slot, sql: string): string | undefined {
  const editor = editorSelection(tabId, slot);
  const target = runTarget(editor?.text ?? sql, editor);
  if ("statement" in target) return target.statement.text;
  toast.error(target.error);
  return undefined;
}

export async function executeTabQuery(
  mode: ExecutionMode = "query",
  scope: ExecutionScope = "statement",
) {
  const store = useTabStore.getState();
  const tab = store.tabs[store.selectedTabIndex];
  const split = mode === "split";
  const slot: Slot = split ? "split" : "main";
  const editorSql = split ? tab?.splitEditorValue : tab?.editorValue;
  if (!tab?.projectId || !editorSql?.trim() || (split && !tab.isSplit)) return;
  const explaining = mode === "explain" || mode === "analyze";
  const sql = explaining
    ? explainTarget(tab.id, editorSql)
    : scope === "script"
      ? editorSql
      : runTargetSql(tab.id, slot, editorSql);
  if (sql === undefined) return;
  const projectId = tab.projectId;
  const project = useProjectStore.getState().projects[projectId];
  if (!project) return;

  const driver = DriverFactory.getDriver(project.driver);
  const execId = crypto.randomUUID();
  const execution = trackExecution(tab.id, slot, () => driver.cancelQuery?.(execId));
  if (split) store.setSplitExecuting(tab.id, true, execId);
  else store.setExecuting(tab.id, true, execId);
  const ownsTab = () => {
    const current = useTabStore.getState().tabs.find((item) => item.id === tab.id);
    return (
      execution.active &&
      current?.projectId === projectId &&
      (split ? current.isSplit && current.splitExecId === execId : current.execId === execId)
    );
  };
  const started = Date.now();
  let queryId: string | undefined;
  let retained = false;
  const release = async () => {
    if (!queryId) return;
    virtualCache.clearQuery(queryId);
    await driver.closeVirtual?.(projectId, queryId);
  };
  try {
    if (useProjectStore.getState().status[projectId] !== "Connected") {
      await useProjectStore.getState().connect(projectId);
      if (!ownsTab()) return;
      if (useProjectStore.getState().status[projectId] !== "Connected") {
        throw new Error("Could not connect to the database. Check the connection and try again.");
      }
    }
    if (!ownsTab()) return;
    const timeout = tab.queryTimeout || undefined;
    releaseResult(tab.id, slot);
    if (split) store.setSplitVirtualQuery(tab.id, undefined);
    else {
      store.setVirtualQuery(tab.id, undefined);
      store.setExplainResult(tab.id, undefined);
      store.discardEditSession(tab.id);
    }

    if (explaining) {
      const plans = await driver.explainPlan(projectId, sql, mode === "analyze", timeout, execId);
      if (!ownsTab()) return;
      if (!Array.isArray(plans) || !plans[0]?.Plan) throw new Error("Invalid EXPLAIN result");
      store.setExplainResult(tab.id, plans[0] as ExplainPlan);
      return;
    }

    let result: QueryResult;
    let rowCount: number;
    if (driver.executeVirtual) {
      queryId = crypto.randomUUID();
      const [header, totalRows, packed, time, capped] = await driver.executeVirtual(
        projectId,
        sql,
        queryId,
        PAGE_SIZE,
        timeout,
        execId,
      );
      if (!ownsTab()) return;
      result = header
        ? { columns: decodeColumns(header), rows: decodePage(packed), time, capped }
        : { ...decodeResult(packed), time, capped };
      rowCount = header ? totalRows : result.rows.length;
      if (header && totalRows > PAGE_SIZE) {
        virtualCache.setPage(queryId, 0, result.rows);
        trackResult(tab.id, slot, release);
        retained = true;
        const virtualQuery = {
          queryId,
          columns: result.columns,
          totalRows,
          pageSize: PAGE_SIZE,
          colCount: result.columns.length,
          time,
        };
        if (split) store.setSplitVirtualQuery(tab.id, virtualQuery);
        else store.setVirtualQuery(tab.id, virtualQuery);
      }
    } else {
      const [columns, rows, time] = await driver.runQuery(projectId, sql, timeout, execId);
      if (!ownsTab()) return;
      result = { columns, rows, time };
      rowCount = rows.length;
    }

    if (!ownsTab()) return;
    if (split) store.setSplitResult(tab.id, result);
    else store.updateResult(tab.id, result);
    notifyQueryComplete(sql, result.time, true, rowCount);
    useHistoryStore.getState().addEntry({
      projectId,
      database: project.database,
      sql: sql.trim(),
      executionTime: result.time,
      rowCount,
      success: true,
      timestamp: started,
    });
  } catch (error) {
    if (!ownsTab()) return;
    const result = executionError(error, Date.now() - started);
    if (split) store.setSplitResult(tab.id, result);
    else store.updateResult(tab.id, result);
    if (result.status !== "cancelled") notifyQueryComplete(sql, result.time, false);
    useHistoryStore.getState().addEntry({
      projectId,
      database: project.database,
      sql: sql.trim(),
      executionTime: result.time,
      rowCount: 0,
      success: false,
      error: result.message,
      timestamp: started,
    });
  } finally {
    if (ownsTab()) {
      if (split) store.setSplitExecuting(tab.id, false);
      else store.setExecuting(tab.id, false);
    }
    execution.finish();
    if (!retained) await release().catch((error) => console.error("Result cleanup failed:", error));
    if (mode !== "explain" && changesSchema(sql))
      useSchemaIndexStore.getState().invalidateProject(projectId);
  }
}

export function cancelTabQuery(split = false) {
  const store = useTabStore.getState();
  const tab = store.tabs[store.selectedTabIndex];
  if (!tab || !(split ? tab.isSplitExecuting : tab.isExecuting)) return;
  cancelExecution(tab.id, split ? "split" : "main");
  const result: QueryResult = {
    columns: [],
    rows: [],
    time: 0,
    status: "cancelled",
    message: "Query cancelled",
  };
  if (split) {
    releaseResult(tab.id, "split");
    store.setSplitVirtualQuery(tab.id, undefined);
    store.setSplitResult(tab.id, result);
  } else {
    releaseResult(tab.id, "main");
    store.setVirtualQuery(tab.id, undefined);
    store.setExplainResult(tab.id, undefined);
    store.updateResult(tab.id, result);
  }
}
