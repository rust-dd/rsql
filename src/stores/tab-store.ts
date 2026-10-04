import { create } from "zustand";
import { persist } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import type { EditSession } from "@/lib/mutations";
import { cancelExecution, releaseTabResources } from "@/lib/tab-resources";
import type { CellValue } from "@/lib/wire";
import type { ExplainPlan, QueryResult, Tab, VirtualQuery } from "@/types";

let nextId = 1;
function genTabId(): string {
  return `tab-${nextId++}-${Date.now()}`;
}

interface TabState {
  tabs: Tab[];
  selectedTabIndex: number;

  openTab: (projectId?: string, editorValue?: string) => void;
  openMonitorTab: (projectId: string) => void;
  openERDTab: (projectId: string, schema: string) => void;
  openTerminalTab: () => void;
  openNotifyTab: (projectId: string) => void;
  openRolesTab: (projectId: string) => void;
  openSchemaDiffTab: (projectId: string) => void;
  openExtensionsTab: (projectId: string) => void;
  openEnumsTab: (projectId: string) => void;
  openPgSettingsTab: (projectId: string) => void;
  closeTab: (index: number) => void;
  closeAllTabs: () => void;
  closeOtherTabs: (index: number) => void;
  selectTab: (index: number) => void;
  updateContent: (tabId: string, value: string) => void;
  updateResult: (tabId: string, result: QueryResult) => void;
  setResult: (tabId: string, result: QueryResult) => void;
  setExecuting: (tabId: string, executing: boolean, execId?: string) => void;
  setProjectId: (tabId: string, projectId: string) => void;
  setExplainResult: (tabId: string, plan: ExplainPlan | undefined) => void;
  setVirtualQuery: (tabId: string, vq: VirtualQuery | undefined) => void;
  toggleSplit: (tabId: string) => void;
  updateSplitContent: (tabId: string, value: string) => void;
  setSplitResult: (tabId: string, result: QueryResult) => void;
  setSplitExecuting: (tabId: string, executing: boolean, execId?: string) => void;
  setQueryTimeout: (tabId: string, timeout: number) => void;

  startEditSession: (tabId: string, session: EditSession) => void;
  discardEditSession: (tabId: string) => void;
  setCellEdit: (tabId: string, key: string, column: string, value: CellValue | undefined) => void;
  setRowDeleted: (tabId: string, key: string, deleted: boolean) => void;
}

/// Tabs are addressed by id, never by index: an index captured before an await
/// can point at a different tab, or past the end, by the time the action runs.
/// A tab that has since been closed makes the action a no-op.
function withTab(state: TabState, tabId: string, apply: (tab: Tab) => void): void {
  const tab = state.tabs.find((t) => t.id === tabId);
  if (tab) apply(tab);
}

function makeSingletonTab(
  type: Tab["type"],
  projectId: string,
  title: string,
  schema?: string,
): (s: TabState) => void {
  return (s) => {
    const existing = s.tabs.findIndex(
      (t) => t.type === type && t.projectId === projectId && (!schema || t.schema === schema),
    );
    if (existing >= 0) {
      s.selectedTabIndex = existing;
      return;
    }
    const newTab: Tab = {
      id: genTabId(),
      type,
      projectId,
      schema,
      title,
      editorValue: "",
      isExecuting: false,
    };
    s.tabs.push(newTab);
    s.selectedTabIndex = s.tabs.length - 1;
  };
}

export const useTabStore = create<TabState>()(
  persist(
    immer((set, get) => ({
      tabs: [
        {
          id: genTabId(),
          type: "query",
          title: "Query 1",
          editorValue: "",
          isExecuting: false,
        },
      ],
      selectedTabIndex: 0,

      openTab: (projectId?: string, editorValue: string = "") => {
        set((s) => {
          s.tabs.push({
            id: genTabId(),
            type: "query",
            projectId,
            title: `Query ${s.tabs.length + 1}`,
            editorValue,
            isExecuting: false,
          });
          s.selectedTabIndex = s.tabs.length - 1;
        });
      },

      openMonitorTab: (projectId) => set(makeSingletonTab("monitor", projectId, "Monitor")),
      openERDTab: (projectId, schema) =>
        set(makeSingletonTab("erd", projectId, `ERD: ${schema}`, schema)),
      openTerminalTab: () => {
        set((s) => {
          s.tabs.push({
            id: genTabId(),
            type: "terminal",
            title: "Terminal",
            editorValue: "",
            isExecuting: false,
          });
          s.selectedTabIndex = s.tabs.length - 1;
        });
      },
      openNotifyTab: (projectId) => set(makeSingletonTab("notify", projectId, "LISTEN/NOTIFY")),
      openRolesTab: (projectId) => set(makeSingletonTab("roles", projectId, "Roles")),
      openSchemaDiffTab: (projectId) =>
        set(makeSingletonTab("schema-diff", projectId, "Schema Diff")),
      openExtensionsTab: (projectId) =>
        set(makeSingletonTab("extensions", projectId, "Extensions")),
      openEnumsTab: (projectId) => set(makeSingletonTab("enums", projectId, "Enum Types")),
      openPgSettingsTab: (projectId) =>
        set(makeSingletonTab("pg-settings", projectId, "PG Settings")),

      closeTab: (index) => {
        const closing = get().tabs[index];
        if (!closing) return;
        releaseTabResources(closing.id);
        set((s) => {
          s.tabs.splice(index, 1);
          if (s.tabs.length === 0) {
            s.selectedTabIndex = -1;
          } else if (s.selectedTabIndex >= s.tabs.length) {
            s.selectedTabIndex = s.tabs.length - 1;
          } else if (s.selectedTabIndex > index) {
            s.selectedTabIndex--;
          } else if (s.selectedTabIndex === index && s.selectedTabIndex > 0) {
            s.selectedTabIndex--;
          }
        });
      },

      closeAllTabs: () => {
        for (const tab of get().tabs) releaseTabResources(tab.id);
        set((s) => {
          s.tabs = [];
          s.selectedTabIndex = -1;
        });
      },

      closeOtherTabs: (index) => {
        if (!get().tabs[index]) return;
        for (const [i, tab] of get().tabs.entries()) {
          if (i !== index) releaseTabResources(tab.id);
        }
        set((s) => {
          const keep = s.tabs[index];
          if (!keep) return;
          s.tabs = [keep];
          s.selectedTabIndex = 0;
        });
      },

      selectTab: (index) =>
        set((s) => {
          s.selectedTabIndex = index;
        }),

      updateContent: (tabId, value) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.editorValue = value;
          });
        }),
      updateResult: (tabId, result) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.result = result;
            tab.isExecuting = false;
            tab.execId = undefined;
          });
        }),
      setResult: (tabId, result) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.result = result;
          });
        }),
      setExecuting: (tabId, executing, execId) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.isExecuting = executing;
            tab.execId = executing ? execId : undefined;
          });
        }),
      setProjectId: (tabId, projectId) => {
        if (get().tabs.find((tab) => tab.id === tabId)?.projectId === projectId) return;
        releaseTabResources(tabId);
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.projectId = projectId;
            tab.result = undefined;
            tab.virtualQuery = undefined;
            tab.explainResult = undefined;
            tab.editSession = undefined;
            tab.isExecuting = false;
            tab.execId = undefined;
            tab.isSplitExecuting = false;
            tab.splitExecId = undefined;
            tab.splitResult = undefined;
          });
        });
      },
      setExplainResult: (tabId, plan) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.explainResult = plan;
          });
        }),
      setVirtualQuery: (tabId, vq) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.virtualQuery = vq;
          });
        }),

      toggleSplit: (tabId) => {
        if (get().tabs.find((tab) => tab.id === tabId)?.isSplit) {
          cancelExecution(tabId, "split");
        }
        set((s) => {
          withTab(s, tabId, (tab) => {
            if (tab.type !== "query") return;
            tab.isSplit = !tab.isSplit;
            if (!tab.isSplit) {
              tab.isSplitExecuting = false;
              tab.splitExecId = undefined;
            }
            tab.splitEditorValue = tab.splitEditorValue ?? "";
          });
        });
      },

      updateSplitContent: (tabId, value) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.splitEditorValue = value;
          });
        }),
      setSplitResult: (tabId, result) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.splitResult = result;
            tab.isSplitExecuting = false;
            tab.splitExecId = undefined;
          });
        }),
      setSplitExecuting: (tabId, executing, execId) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.isSplitExecuting = executing;
            tab.splitExecId = executing ? execId : undefined;
          });
        }),
      setQueryTimeout: (tabId, timeout) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.queryTimeout = timeout;
          });
        }),

      startEditSession: (tabId, session) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.editSession = session;
          });
        }),

      discardEditSession: (tabId) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            tab.editSession = undefined;
          });
        }),

      // `undefined` means the cell was returned to its original value, so the
      // pending edit is dropped rather than recorded as a no-op assignment.
      setCellEdit: (tabId, key, column, value) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            const session = tab.editSession;
            if (!session) return;
            if (value === undefined) {
              const columns = session.edits[key];
              if (!columns) return;
              delete columns[column];
              if (Object.keys(columns).length === 0) delete session.edits[key];
              return;
            }
            session.edits[key] ??= {};
            session.edits[key][column] = value;
          });
        }),

      setRowDeleted: (tabId, key, deleted) =>
        set((s) => {
          withTab(s, tabId, (tab) => {
            const session = tab.editSession;
            if (!session) return;
            const marked = session.deletes.includes(key);
            if (deleted && !marked) session.deletes.push(key);
            if (!deleted && marked) {
              session.deletes = session.deletes.filter((k) => k !== key);
            }
          });
        }),
    })),
    {
      name: "rsql-tabs",
      partialize: (state) => ({
        tabs: state.tabs
          .filter((tab) => tab.type !== "terminal" && tab.type !== "notify")
          .map((tab) => ({
            id: tab.id,
            type: tab.type,
            projectId: tab.projectId,
            schema: tab.schema,
            title: tab.title,
            editorValue: tab.editorValue,
            isExecuting: false,
            queryTimeout: tab.queryTimeout,
            isSplit: tab.isSplit,
            splitEditorValue: tab.splitEditorValue,
          })),
        selectedTabIndex:
          state.tabs.filter((t) => t.type !== "terminal").length === 0
            ? -1
            : Math.min(
                state.selectedTabIndex,
                state.tabs.filter((t) => t.type !== "terminal").length - 1,
              ),
      }),
      merge: (persisted: unknown, current: TabState) => {
        const p = persisted as Partial<TabState> | undefined;
        if (!p?.tabs || !Array.isArray(p.tabs)) return current;
        if (p.tabs.length === 0) return { ...current, tabs: [], selectedTabIndex: -1 };
        const validTabs = p.tabs.filter(
          (t): t is Tab =>
            t != null && typeof t === "object" && "id" in t && "type" in t && "title" in t,
        );
        if (validTabs.length === 0) return { ...current, tabs: [], selectedTabIndex: -1 };
        const idx = Math.min(Math.max(0, p.selectedTabIndex ?? 0), validTabs.length - 1);
        return { ...current, tabs: validTabs, selectedTabIndex: idx };
      },
    },
  ),
);

export function useActiveTab(): Tab | undefined {
  return useTabStore((s) => s.tabs[s.selectedTabIndex]);
}

export function useActiveTabId(): string | undefined {
  return useTabStore((s) => s.tabs[s.selectedTabIndex]?.id);
}

/// Id of a tab by position, for the few callers that still hold an index.
export function tabIdAt(index: number): string | undefined {
  return useTabStore.getState().tabs[index]?.id;
}
