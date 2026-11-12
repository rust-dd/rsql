import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import {
  type HistoryStatus,
  historyClear,
  historyRecord,
  historySetEnabled,
  historyStatus,
} from "@/tauri";

export interface HistoryEntry {
  id: string;
  projectId: string;
  database: string;
  sql: string;
  executionTime: number;
  rowCount: number;
  success: boolean;
  error?: string;
  timestamp: number;
}

interface HistoryState {
  /** This session's executions, for the performance monitor. */
  entries: HistoryEntry[];
  /** The history saved on this device; null until loaded. */
  status: HistoryStatus | null;
  /** Bumped whenever the saved history changes, so views can reload it. */
  revision: number;
  addEntry: (entry: Omit<HistoryEntry, "id">) => void;
  refreshStatus: () => Promise<void>;
  setEnabled: (enabled: boolean) => Promise<void>;
  clearHistory: () => Promise<void>;
}

let historyId = 0;

export const useHistoryStore = create<HistoryState>()(
  immer((set, get) => ({
    entries: [],
    status: null,
    revision: 0,

    addEntry: (entry) => {
      set((s) => {
        s.entries.unshift({ ...entry, id: `hist-${++historyId}` });
        s.entries.length = Math.min(s.entries.length, 500);
      });
      // Not awaited: saving history must never hold up the query that produced it.
      void historyRecord({
        projectId: entry.projectId,
        database: entry.database,
        sql: entry.sql,
        executedAt: entry.timestamp,
        durationMs: entry.executionTime,
        rowCount: entry.rowCount,
        success: entry.success,
        error: entry.error,
      })
        .then(async (saved) => {
          if (!saved) return;
          set((s) => {
            s.revision += 1;
          });
          await get().refreshStatus();
        })
        .catch((error) => console.error("Saving query history failed:", error));
    },

    refreshStatus: async () => {
      const status = await historyStatus();
      set({ status });
    },

    setEnabled: async (enabled) => {
      await historySetEnabled(enabled);
      await get().refreshStatus();
    },

    clearHistory: async () => {
      await historyClear();
      set((s) => {
        s.entries = [];
        s.revision += 1;
      });
      await get().refreshStatus();
    },
  })),
);
