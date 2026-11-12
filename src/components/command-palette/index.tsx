import { Command } from "cmdk";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { format as formatSQL } from "sql-formatter";
import { useProjectStore } from "@/stores/project-store";
import { useQueryStore } from "@/stores/query-store";
import { useActiveTab, useTabStore } from "@/stores/tab-store";
import { useUIStore } from "@/stores/ui-store";
import { useWorkspaceStore } from "@/stores/workspace-store";
import { SaveQueryPage } from "./queries";
import { ConnectionsGroup, DatabaseObjectsGroups } from "./search";
import { ActionsGroup } from "./tools";
import type { Page } from "./types";
import { LoadOrDeleteWorkspacePage, SaveWorkspacePage, WorkspacesGroup } from "./workspaces";

export function CommandPalette({
  open,
  onClose,
  onExecute,
  onExecuteScript,
  onExplain,
  onAnalyze,
  onCheckUpdates,
}: {
  open: boolean;
  onClose: () => void;
  onExecute: () => void;
  onExecuteScript: () => void;
  onExplain: () => void;
  onAnalyze: () => void;
  onCheckUpdates: () => void;
}) {
  const [page, setPage] = useState<Page>("root");
  const [workspaceName, setWorkspaceName] = useState("");
  const [queryName, setQueryName] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);

  const tables = useProjectStore((s) => s.tables);
  const views = useProjectStore((s) => s.views);
  const materializedViews = useProjectStore((s) => s.materializedViews);
  const functions = useProjectStore((s) => s.functions);
  const schemas = useProjectStore((s) => s.schemas);
  const projects = useProjectStore((s) => s.projects);
  const status = useProjectStore((s) => s.status);
  const connectProject = useProjectStore((s) => s.connect);

  const openTab = useTabStore((s) => s.openTab);
  const activeTab = useActiveTab();

  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const setConnectionModalOpen = useUIStore((s) => s.setConnectionModalOpen);
  const pinnedResult = useUIStore((s) => s.pinnedResult);
  const clearPinnedResult = useUIStore((s) => s.clearPinnedResult);

  const workspaces = useWorkspaceStore((s) => s.workspaces);
  const loadWorkspaces = useWorkspaceStore((s) => s.load);
  const saveWorkspace = useWorkspaceStore((s) => s.save);
  const removeWorkspace = useWorkspaceStore((s) => s.remove);
  const workspacesLoaded = useWorkspaceStore((s) => s.loaded);

  const saveQueryAction = useQueryStore((s) => s.saveQuery);

  useEffect(() => {
    if (open) {
      setPage("root");
      setWorkspaceName("");
      setQueryName("");
      if (!workspacesLoaded) void loadWorkspaces();
    }
  }, [open, workspacesLoaded, loadWorkspaces]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (page !== "root") {
          e.preventDefault();
          e.stopPropagation();
          setPage("root");
          setWorkspaceName("");
          setQueryName("");
        } else {
          onClose();
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, page, onClose]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [open, onClose]);

  const handleSaveWorkspace = useCallback(async () => {
    if (!workspaceName.trim()) return;
    const tabs = useTabStore
      .getState()
      .tabs.filter((t) => t.type === "query")
      .map((t) => ({
        title: t.title,
        editorValue: t.editorValue,
        projectId: t.projectId,
        type: t.type,
      }));
    await saveWorkspace(workspaceName.trim(), JSON.stringify(tabs));
    onClose();
  }, [workspaceName, saveWorkspace, onClose]);

  const handleSaveQuery = useCallback(async () => {
    if (!queryName.trim()) return;
    const tab = useTabStore.getState().tabs[useTabStore.getState().selectedTabIndex];
    if (!tab?.projectId || !tab.editorValue?.trim()) return;
    const d = useProjectStore.getState().projects[tab.projectId];
    if (!d) return;
    await saveQueryAction(tab.projectId, d.database, d.driver, queryName.trim(), tab.editorValue);
    onClose();
  }, [queryName, saveQueryAction, onClose]);

  const handleLoadWorkspace = useCallback(
    (tabsJson: string) => {
      try {
        const tabs = JSON.parse(tabsJson) as {
          title: string;
          editorValue: string;
          projectId?: string;
          type: string;
        }[];
        const store = useTabStore.getState();
        for (const tab of tabs) {
          store.openTab(tab.projectId, tab.editorValue);
        }
      } catch {
        /* ignore parse errors */
      }
      onClose();
    },
    [onClose],
  );

  const handleDeleteWorkspace = useCallback(
    async (name: string) => {
      await removeWorkspace(name);
      if (workspaces.length <= 1) setPage("root");
    },
    [removeWorkspace, workspaces.length],
  );

  const selectItem = useCallback(
    (type: string, projectId: string, schema: string, name: string) => {
      onClose();
      if (type === "table" || type === "view" || type === "matview") {
        openTab(projectId, `SELECT * FROM "${schema}"."${name}" LIMIT 100;`);
      } else if (type === "function") {
        openTab(
          projectId,
          `-- Function: ${schema}.${name}\nSELECT pg_get_functiondef(p.oid)\nFROM pg_proc p\nJOIN pg_namespace n ON n.oid = p.pronamespace\nWHERE n.nspname = '${schema}' AND p.proname = '${name}'\nLIMIT 1;`,
        );
      } else if (type === "schema") {
        openTab(projectId, `-- Schema: ${name}\n`);
      }
    },
    [openTab, onClose],
  );

  const formatQuery = useCallback(() => {
    const { tabs, selectedTabIndex: idx } = useTabStore.getState();
    const tab = tabs[idx];
    if (!tab?.editorValue?.trim()) return;
    try {
      const formatted = formatSQL(tab.editorValue, {
        language: "postgresql",
        tabWidth: 2,
        keywordCase: "upper",
      });
      const tabId = useTabStore.getState().tabs[idx]?.id;
      if (tabId) useTabStore.getState().updateContent(tabId, formatted);
    } catch {
      /* ignore */
    }
    onClose();
  }, [onClose]);

  if (!open) return null;

  const activeProject = activeTab?.projectId;
  const hasQuery = !!activeTab?.editorValue?.trim();

  return createPortal(
    <>
      <div className="cmdk-overlay" />
      <div ref={containerRef} className="cmdk-content">
        <Command
          label="Command palette"
          onKeyDown={(e) => {
            if (e.key === "Backspace" && page !== "root") {
              const input = e.currentTarget.querySelector(
                "[cmdk-input]",
              ) as HTMLInputElement | null;
              if (input && input.value === "") {
                e.preventDefault();
                setPage("root");
                setWorkspaceName("");
                setQueryName("");
              }
            }
          }}
          loop
        >
          {page === "save-workspace" ? (
            <SaveWorkspacePage
              workspaceName={workspaceName}
              setWorkspaceName={setWorkspaceName}
              setPage={setPage}
              handleSaveWorkspace={handleSaveWorkspace}
            />
          ) : page === "save-query" ? (
            <SaveQueryPage
              queryName={queryName}
              setQueryName={setQueryName}
              setPage={setPage}
              handleSaveQuery={handleSaveQuery}
            />
          ) : page === "load-workspace" || page === "delete-workspace" ? (
            <LoadOrDeleteWorkspacePage
              page={page}
              workspaces={workspaces}
              handleDeleteWorkspace={handleDeleteWorkspace}
              handleLoadWorkspace={handleLoadWorkspace}
            />
          ) : (
            <>
              <Command.Input placeholder="Search commands, tables, views..." autoFocus />
              <Command.List>
                <Command.Empty>No results found</Command.Empty>

                <ActionsGroup
                  onClose={onClose}
                  setConnectionModalOpen={setConnectionModalOpen}
                  activeProject={activeProject}
                  hasQuery={hasQuery}
                  onExecute={onExecute}
                  onExecuteScript={onExecuteScript}
                  onExplain={onExplain}
                  onAnalyze={onAnalyze}
                  formatQuery={formatQuery}
                  setPage={setPage}
                  theme={theme}
                  toggleTheme={toggleTheme}
                  onCheckUpdates={onCheckUpdates}
                  pinnedResult={pinnedResult}
                  clearPinnedResult={clearPinnedResult}
                  activeTabResult={!!activeTab?.result}
                />

                <ConnectionsGroup
                  projects={projects}
                  status={status}
                  schemas={schemas}
                  onClose={onClose}
                  connectProject={connectProject}
                />

                <WorkspacesGroup setPage={setPage} workspaces={workspaces} />

                <DatabaseObjectsGroups
                  tables={tables}
                  views={views}
                  materializedViews={materializedViews}
                  functions={functions}
                  schemas={schemas}
                  selectItem={selectItem}
                />
              </Command.List>
            </>
          )}
        </Command>
      </div>
    </>,
    document.body,
  );
}
