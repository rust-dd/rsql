import { lazy, Suspense, useCallback, useState } from "react";
import { Toaster } from "sonner";
import { EditorToolbar } from "@/components/editor-toolbar";
import { QueryEditor } from "@/components/query-editor";
import { ResizeHandle } from "@/components/resize-handle";
import { ResultsGrid } from "@/components/results-grid";
import { ResultsPanel } from "@/components/results-panel";
import { QueryFeedback } from "@/components/results-panel/feedback";
import { ServerSidebar } from "@/components/server-sidebar";
import { StatusBar } from "@/components/status-bar";
import { TabBar } from "@/components/tab-bar";
import { TopBar } from "@/components/top-bar";
import { useAppStartup } from "@/hooks/use-app-startup";
import { useQueryLifecycle } from "@/hooks/use-query-lifecycle";
import { checkForUpdates } from "@/lib/updater";
import { useProjectStore } from "@/stores/project-store";
import { useActiveTab, useActiveTabId, useTabStore } from "@/stores/tab-store";
import { useUIStore } from "@/stores/ui-store";
import type { ProjectDetails } from "@/types";
import "@/monaco/setup";

const CommandPalette = lazy(() =>
  import("@/components/command-palette").then((module) => ({ default: module.CommandPalette })),
);
const ConnectionModal = lazy(() =>
  import("@/components/connection-modal").then((module) => ({ default: module.ConnectionModal })),
);
const EnumsPanel = lazy(() =>
  import("@/components/enums-panel").then((module) => ({ default: module.EnumsPanel })),
);
const ERDDiagram = lazy(() =>
  import("@/components/erd-diagram").then((module) => ({ default: module.ERDDiagram })),
);
const ExtensionsPanel = lazy(() =>
  import("@/components/extensions-panel").then((module) => ({ default: module.ExtensionsPanel })),
);
const NotifyPanel = lazy(() =>
  import("@/components/notify-panel").then((module) => ({ default: module.NotifyPanel })),
);
const PerformanceMonitor = lazy(() =>
  import("@/components/performance-monitor").then((module) => ({
    default: module.PerformanceMonitor,
  })),
);
const PgSettingsPanel = lazy(() =>
  import("@/components/pg-settings-panel").then((module) => ({ default: module.PgSettingsPanel })),
);
const RolesPanel = lazy(() =>
  import("@/components/roles-panel").then((module) => ({ default: module.RolesPanel })),
);
const SchemaDiffPanel = lazy(() =>
  import("@/components/schema-diff-panel").then((module) => ({ default: module.SchemaDiffPanel })),
);
const TerminalPanel = lazy(() =>
  import("@/components/terminal-panel").then((module) => ({ default: module.TerminalPanel })),
);

export default function App() {
  const sidebarWidth = useUIStore((s) => s.sidebarWidth);
  const editorHeight = useUIStore((s) => s.editorHeight);
  const connectionModalOpen = useUIStore((s) => s.connectionModalOpen);
  const setConnectionModalOpen = useUIStore((s) => s.setConnectionModalOpen);
  const setSidebarWidth = useUIStore((s) => s.setSidebarWidth);
  const setEditorHeight = useUIStore((s) => s.setEditorHeight);

  const projects = useProjectStore((s) => s.projects);
  const saveConnection = useProjectStore((s) => s.saveConnection);
  const updateConnection = useProjectStore((s) => s.updateConnection);
  const activeTabId = useActiveTabId();
  const activeTab = useActiveTab();
  const updateContent = useTabStore((s) => s.updateContent);

  const [editingConnection, setEditingConnection] = useState<{
    name: string;
    details: ProjectDetails;
  } | null>(null);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);

  useAppStartup();
  const { runQuery, runExplain, cancelQuery, runSplitQuery, cancelSplitQuery } = useQueryLifecycle({
    setCommandPaletteOpen,
  });

  const handleSaveConnection = useCallback(
    async (connection: {
      name: string;
      driver: string;
      username: string;
      password: string;
      database: string;
      host: string;
      port: string;
      ssl: boolean;
      sshEnabled?: boolean;
      sshHost?: string;
      sshPort?: string;
      sshUser?: string;
      sshPassword?: string;
      sshKeyPath?: string;
    }) => {
      const details = {
        driver: connection.driver as "PGSQL",
        username: connection.username,
        password: connection.password,
        database: connection.database,
        host: connection.host,
        port: connection.port,
        ssl: connection.ssl ? "true" : "false",
        sshEnabled: connection.sshEnabled ? "true" : "false",
        sshHost: connection.sshHost ?? "",
        sshPort: connection.sshPort ?? "22",
        sshUser: connection.sshUser ?? "",
        sshPassword: connection.sshPassword ?? "",
        sshKeyPath: connection.sshKeyPath ?? "",
      };
      if (editingConnection) {
        await updateConnection(connection.name, details);
        setEditingConnection(null);
      } else {
        await saveConnection(connection.name, details);
      }
    },
    [saveConnection, updateConnection, editingConnection],
  );

  const handleEditConnection = useCallback(
    (projectId: string) => {
      const details = projects[projectId];
      if (details) {
        setEditingConnection({ name: projectId, details });
        setConnectionModalOpen(true);
      }
    },
    [projects, setConnectionModalOpen],
  );

  const handleModalClose = useCallback(
    (open: boolean) => {
      if (!open) setEditingConnection(null);
      setConnectionModalOpen(open);
    },
    [setConnectionModalOpen],
  );

  return (
    <div
      className="flex h-screen flex-col bg-background text-foreground"
      onContextMenu={(e) => e.preventDefault()}
    >
      <TopBar
        onCheckUpdates={() => void checkForUpdates()}
        onOpenCommandPalette={() => setCommandPaletteOpen(true)}
      />

      <div className="flex flex-1 overflow-hidden">
        <div
          style={{ width: `${sidebarWidth}px`, minWidth: "180px" }}
          className="flex-shrink-0 overflow-hidden"
        >
          <ServerSidebar onEditConnection={handleEditConnection} />
        </div>
        <ResizeHandle direction="horizontal" onResize={setSidebarWidth} />

        <div className="flex flex-1 flex-col overflow-hidden">
          <TabBar />
          <Suspense
            fallback={
              <div
                role="status"
                className="flex flex-1 items-center justify-center text-sm text-muted-foreground"
              >
                Loading panel…
              </div>
            }
          >
            {!activeTab ? (
              <div className="flex-1 flex items-center justify-center">
                <div className="text-center space-y-4">
                  <div className="text-muted-foreground/40 text-6xl font-mono font-bold select-none">
                    RSQL
                  </div>
                  <p className="text-muted-foreground/60 text-sm">No tabs open</p>
                  <button
                    type="button"
                    onClick={() => useTabStore.getState().openTab()}
                    className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-primary/10 text-primary hover:bg-primary/20 transition-colors text-sm font-medium"
                  >
                    <span className="text-lg leading-none">+</span> New Query
                  </button>
                </div>
              </div>
            ) : activeTab?.type === "monitor" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <PerformanceMonitor projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.type === "erd" && activeTab.projectId && activeTab.schema ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <ERDDiagram projectId={activeTab.projectId} schema={activeTab.schema} />
              </div>
            ) : activeTab?.type === "terminal" ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <TerminalPanel terminalId={activeTab.id} />
              </div>
            ) : activeTab?.type === "notify" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <NotifyPanel projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.type === "roles" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <RolesPanel projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.type === "schema-diff" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <SchemaDiffPanel projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.type === "extensions" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <ExtensionsPanel projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.type === "enums" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <EnumsPanel projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.type === "pg-settings" && activeTab.projectId ? (
              <div className="flex-1 min-h-0 overflow-hidden">
                <PgSettingsPanel projectId={activeTab.projectId} />
              </div>
            ) : activeTab?.isSplit ? (
              <>
                <EditorToolbar
                  onExecute={() => void runQuery()}
                  onExplain={() => void runExplain()}
                  onCancel={() => void cancelQuery()}
                />
                <div className="flex flex-1 min-h-0 overflow-hidden">
                  {/* Left pane */}
                  <div className="flex flex-1 flex-col overflow-hidden border-r border-border/30">
                    <div
                      style={{ height: `${editorHeight}%` }}
                      className="flex flex-col overflow-hidden"
                    >
                      <QueryEditor
                        value={activeTab.editorValue}
                        onChange={(v) => activeTabId && updateContent(activeTabId, v)}
                        onExecute={() => void runQuery()}
                        onExplain={() => void runExplain()}
                      />
                    </div>
                    <ResizeHandle direction="vertical" onResize={setEditorHeight} />
                    <div className="flex-1 min-h-0">
                      <ResultsPanel />
                    </div>
                  </div>
                  {/* Right pane */}
                  <div className="flex flex-1 flex-col overflow-hidden">
                    <div className="flex items-center justify-between border-b border-border px-3 py-1 text-xs">
                      <span className="text-muted-foreground">Right query</span>
                      <button
                        type="button"
                        onClick={() =>
                          activeTab.isSplitExecuting ? cancelSplitQuery() : void runSplitQuery()
                        }
                        disabled={!activeTab.projectId || !activeTab.splitEditorValue?.trim()}
                        className="rounded border border-border px-2 py-1 hover:bg-accent disabled:opacity-40"
                      >
                        {activeTab.isSplitExecuting ? "Stop right query" : "Run right query"}
                      </button>
                    </div>
                    <div
                      style={{ height: `${editorHeight}%` }}
                      className="flex flex-col overflow-hidden"
                    >
                      <QueryEditor
                        value={activeTab.splitEditorValue ?? ""}
                        onChange={(v) =>
                          activeTabId && useTabStore.getState().updateSplitContent(activeTabId, v)
                        }
                        onExecute={() => void runSplitQuery()}
                      />
                    </div>
                    <ResizeHandle direction="vertical" onResize={setEditorHeight} />
                    <div className="flex-1 min-h-0 flex flex-col">
                      {activeTab.isSplitExecuting ? (
                        <div className="flex-1 flex items-center justify-center text-muted-foreground">
                          <span className="animate-spin mr-2">⏳</span> Running...
                        </div>
                      ) : activeTab.splitResult?.status &&
                        activeTab.splitResult.status !== "success" ? (
                        <QueryFeedback
                          result={activeTab.splitResult}
                          onRetry={() => void runSplitQuery()}
                        />
                      ) : activeTab.splitResult ? (
                        <div className="flex-1 flex flex-col overflow-hidden">
                          <div className="flex items-center gap-2 px-3 py-1 border-b border-border/30 text-xs font-mono text-muted-foreground">
                            <span>{activeTab.splitResult.rows.length} rows</span>
                            {activeTab.splitResult.time > 0 && (
                              <span>· {activeTab.splitResult.time.toFixed(1)}ms</span>
                            )}
                          </div>
                          <div className="flex flex-1 min-h-0 flex-col">
                            <ResultsGrid
                              columns={activeTab.splitResult.columns}
                              rows={activeTab.splitResult.rows}
                            />
                          </div>
                        </div>
                      ) : (
                        <div className="flex-1 flex items-center justify-center text-muted-foreground/40 text-sm font-mono">
                          Run a query to see results
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              </>
            ) : (
              <>
                <EditorToolbar
                  onExecute={() => void runQuery()}
                  onExplain={() => void runExplain()}
                  onCancel={() => void cancelQuery()}
                />
                <div
                  style={{ height: `${editorHeight}%` }}
                  className="flex flex-col overflow-hidden"
                >
                  <QueryEditor
                    value={activeTab?.editorValue ?? ""}
                    onChange={(v) => activeTabId && updateContent(activeTabId, v)}
                    onExecute={() => void runQuery()}
                    onExplain={() => void runExplain()}
                  />
                </div>
                <ResizeHandle direction="vertical" onResize={setEditorHeight} />
                <div className="flex-1 min-h-0">
                  <ResultsPanel />
                </div>
              </>
            )}
          </Suspense>
        </div>
      </div>

      <StatusBar />

      <Suspense fallback={null}>
        {connectionModalOpen && (
          <ConnectionModal
            open={connectionModalOpen}
            onOpenChange={handleModalClose}
            onSave={handleSaveConnection}
            editData={editingConnection}
          />
        )}

        {commandPaletteOpen && (
          <CommandPalette
            open={commandPaletteOpen}
            onClose={() => setCommandPaletteOpen(false)}
            onExecute={() => void runQuery()}
            onExplain={() => void runExplain()}
            onCheckUpdates={() => void checkForUpdates()}
          />
        )}
      </Suspense>
      <Toaster theme="dark" position="bottom-right" richColors />
    </div>
  );
}
