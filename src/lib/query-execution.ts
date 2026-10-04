import { DriverFactory } from "@/lib/database-driver";
import { changesSchema } from "@/lib/ddl-detect";
import { isQueryCancelledError, notifyQueryComplete, PAGE_SIZE } from "@/lib/query-helpers";
import { cancelExecution, releaseResult, trackExecution, trackResult } from "@/lib/tab-resources";
import * as virtualCache from "@/lib/virtual-cache";
import { decodeColumns, decodePage, decodeResult } from "@/lib/wire";
import { useHistoryStore } from "@/stores/history-store";
import { useProjectStore } from "@/stores/project-store";
import { useSchemaIndexStore } from "@/stores/schema-index-store";
import { useTabStore } from "@/stores/tab-store";
import type { ExplainPlan, QueryResult } from "@/types";

export type ExecutionMode = "query" | "explain" | "split";

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

export async function executeTabQuery(mode: ExecutionMode = "query") {
  const store = useTabStore.getState();
  const tab = store.tabs[store.selectedTabIndex];
  const sql = mode === "split" ? tab?.splitEditorValue : tab?.editorValue;
  if (!tab?.projectId || !sql?.trim() || (mode === "split" && !tab.isSplit)) return;
  const projectId = tab.projectId;
  const project = useProjectStore.getState().projects[projectId];
  if (!project) return;

  const driver = DriverFactory.getDriver(project.driver);
  const execId = crypto.randomUUID();
  const split = mode === "split";
  const execution = trackExecution(tab.id, split ? "split" : "main", () =>
    driver.cancelQuery?.(execId),
  );
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
    if (!split) {
      releaseResult(tab.id);
      store.setVirtualQuery(tab.id, undefined);
      store.setExplainResult(tab.id, undefined);
      store.discardEditSession(tab.id);
    }

    if (mode === "explain") {
      const [, rows] = await driver.runQuery(
        projectId,
        `EXPLAIN (ANALYZE, FORMAT JSON) ${sql.replace(/;\s*$/, "")}`,
        timeout,
        execId,
      );
      if (!ownsTab()) return;
      const plans: unknown = JSON.parse(rows.map((row) => row[0]).join("\n"));
      if (!Array.isArray(plans) || !plans[0]?.Plan) throw new Error("Invalid EXPLAIN result");
      store.setExplainResult(tab.id, plans[0] as ExplainPlan);
      return;
    }

    let result: QueryResult;
    let rowCount: number;
    if (!split && driver.executeVirtual) {
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
        trackResult(tab.id, release);
        retained = true;
        store.setVirtualQuery(tab.id, {
          queryId,
          columns: result.columns,
          totalRows,
          pageSize: PAGE_SIZE,
          colCount: result.columns.length,
          time,
        });
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
    if (changesSchema(sql)) useSchemaIndexStore.getState().invalidateProject(projectId);
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
  if (split) store.setSplitResult(tab.id, result);
  else {
    releaseResult(tab.id);
    store.setVirtualQuery(tab.id, undefined);
    store.setExplainResult(tab.id, undefined);
    store.updateResult(tab.id, result);
  }
}
