import { useCallback, useMemo, useState } from "react";
import { parseCellInput } from "@/lib/cell-input";
import { DriverFactory } from "@/lib/database-driver";
import { buildMutations, countPending, emptySession, rowKeys } from "@/lib/mutations";
import { parseSelectTable } from "@/lib/sql-utils";
import type { CellValue } from "@/lib/wire";
import { useProjectStore } from "@/stores/project-store";
import { useTabStore } from "@/stores/tab-store";
import type { QueryResult } from "@/types";

interface UseEditModeArgs {
  tabId: string | undefined;
  projectId: string | undefined;
  editorValue: string | undefined;
  result: QueryResult | null | undefined;
}

export function useEditMode({ tabId, projectId, editorValue, result }: UseEditModeArgs) {
  const [editError, setEditError] = useState<string | null>(null);
  const [isCommitting, setIsCommitting] = useState(false);
  const [confirmingApply, setConfirmingApply] = useState(false);

  const editSession = useTabStore((s) => s.tabs.find((t) => t.id === tabId)?.editSession);
  const isEditing = !!editSession;

  const editableTable = useMemo(() => {
    if (!editorValue) return null;
    return parseSelectTable(editorValue);
  }, [editorValue]);

  // A session belongs to the table it was opened against. If the editor now
  // points somewhere else, applying it would write to the wrong table.
  const sessionMatchesEditor =
    !editSession ||
    (!!editableTable &&
      editableTable.schema === editSession.schema &&
      editableTable.table === editSession.table);

  const keys = useMemo(
    () =>
      editSession && result
        ? rowKeys(result.columns, result.rows, editSession.pkColumns)
        : ([] as (string | null)[]),
    [editSession, result],
  );

  const pending = useMemo(
    () => (editSession ? countPending(editSession) : { updates: 0, deletes: 0 }),
    [editSession],
  );

  const handleEnterEdit = useCallback(async () => {
    if (!editableTable || !projectId || !tabId) return;
    const d = useProjectStore.getState().projects[projectId];
    if (!d) return;
    setEditError(null);

    try {
      const driver = DriverFactory.getDriver(d.driver);
      const indexes = await driver.loadIndexes(
        projectId,
        editableTable.schema,
        editableTable.table,
      );
      const pkColumns = [...new Set(indexes.filter((i) => i.isPrimary).map((i) => i.columnName))];

      if (pkColumns.length === 0) {
        setEditError("No primary key found. Inline editing requires a primary key.");
        return;
      }

      const resultCols = result?.columns ?? [];
      const missingPKs = pkColumns.filter((pk) => !resultCols.includes(pk));
      if (missingPKs.length > 0) {
        setEditError(
          `Primary key column(s) ${missingPKs.join(", ")} not in query results. Select all PK columns to edit.`,
        );
        return;
      }

      useTabStore
        .getState()
        .startEditSession(
          tabId,
          emptySession(editableTable.schema, editableTable.table, pkColumns),
        );
    } catch (err: any) {
      setEditError(err?.message ?? "Failed to load table info");
    }
  }, [editableTable, projectId, tabId, result?.columns]);

  const handleDiscard = useCallback(() => {
    if (!tabId) return;
    useTabStore.getState().discardEditSession(tabId);
    setEditError(null);
    setConfirmingApply(false);
  }, [tabId]);

  const handleRequestApply = useCallback(() => {
    if (pending.updates + pending.deletes === 0) return;
    setConfirmingApply(true);
  }, [pending]);

  const handleCancelApply = useCallback(() => setConfirmingApply(false), []);

  const handleConfirmApply = useCallback(async () => {
    setConfirmingApply(false);
    if (!editSession || !projectId || !tabId) return;

    const mutations = buildMutations(editSession);
    if (mutations.length === 0) {
      handleDiscard();
      return;
    }

    setIsCommitting(true);
    setEditError(null);

    try {
      const d = useProjectStore.getState().projects[projectId];
      if (!d) throw new Error("Project not found");
      const driver = DriverFactory.getDriver(d.driver);
      if (!driver.applyRowMutations) throw new Error("Driver does not support inline editing");

      // One transaction for updates and deletes together. The backend requires
      // each statement to affect exactly one row and rolls the batch back
      // otherwise, so a no-op surfaces as an error rather than a silent success.
      await driver.applyRowMutations(
        projectId,
        editSession.schema,
        editSession.table,
        mutations,
        30000,
      );

      const [cols, rows, time] = await driver.runQuery(projectId, editorValue ?? "");
      useTabStore.getState().updateResult(tabId, { columns: cols, rows, time });

      useTabStore.getState().discardEditSession(tabId);
    } catch (err: any) {
      setEditError(err?.message ?? String(err));
    } finally {
      setIsCommitting(false);
    }
  }, [editSession, projectId, tabId, editorValue, handleDiscard]);

  const handleCellEdit = useCallback(
    (rowIndex: number, colIndex: number, value: string) => {
      if (!tabId || !editSession || !result) return;
      const key = keys[rowIndex];
      if (!key) {
        setEditError("This row cannot be identified by its primary key and cannot be edited.");
        return;
      }
      const column = result.columns[colIndex];
      if (!column) return;

      const original = result.rows[rowIndex]?.[colIndex];
      const parsed = parseCellInput(value);
      const next: CellValue | undefined = parsed === original ? undefined : parsed;
      useTabStore.getState().setCellEdit(tabId, key, column, next);
    },
    [tabId, editSession, result, keys],
  );

  const setRowDeleted = useCallback(
    (rowIndex: number, deleted: boolean) => {
      if (!tabId || !editSession) return;
      const key = keys[rowIndex];
      if (!key) {
        setEditError("This row cannot be identified by its primary key and cannot be deleted.");
        return;
      }
      useTabStore.getState().setRowDeleted(tabId, key, deleted);
    },
    [tabId, editSession, keys],
  );

  const handleRowDelete = useCallback(
    (rowIndex: number) => setRowDeleted(rowIndex, true),
    [setRowDeleted],
  );
  const handleRowRestore = useCallback(
    (rowIndex: number) => setRowDeleted(rowIndex, false),
    [setRowDeleted],
  );

  // The grid still works in row positions, so translate identities back to it.
  const editedCells = useMemo(() => {
    const map = new Map<string, CellValue>();
    if (!editSession || !result) return map;
    keys.forEach((key, rowIndex) => {
      if (!key) return;
      const columns = editSession.edits[key];
      if (!columns) return;
      for (const [column, value] of Object.entries(columns)) {
        const colIndex = result.columns.indexOf(column);
        if (colIndex >= 0) map.set(`${rowIndex}:${colIndex}`, value);
      }
    });
    return map;
  }, [editSession, result, keys]);

  const deletedRowIndices = useMemo(() => {
    const set = new Set<number>();
    if (!editSession) return set;
    const marked = new Set(editSession.deletes);
    keys.forEach((key, rowIndex) => {
      if (key && marked.has(key)) set.add(rowIndex);
    });
    return set;
  }, [editSession, keys]);

  return {
    isEditing,
    editSession,
    editError,
    setEditError,
    isCommitting,
    confirmingApply,
    pending,
    sessionMatchesEditor,
    editableTable,
    editedCells,
    deletedRowIndices,
    handleEnterEdit,
    handleDiscard,
    handleRequestApply,
    handleConfirmApply,
    handleCancelApply,
    handleCellEdit,
    handleRowDelete,
    handleRowRestore,
  };
}
