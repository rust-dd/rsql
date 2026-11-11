import { Plus } from "lucide-react";
import React from "react";
import { CSVImportModal } from "@/components/csv-import-modal";
import { ObjectPropertiesModal } from "@/components/object-properties-modal";
import { Button } from "@/components/ui/button";
import { ContextMenu, useContextMenu } from "@/components/ui/context-menu";
import { useProjectStore } from "@/stores/project-store";
import { useQueryStore } from "@/stores/query-store";
import { useTabStore } from "@/stores/tab-store";
import { useUIStore } from "@/stores/ui-store";
import type { ProjectDetails } from "@/types";
import { AddDatabaseDialog } from "./add-database-dialog";
import { serverGroupAlias } from "./labels";
import { RenameServerDialog } from "./rename-server-dialog";
import { renderSavedQueries } from "./render-saved-queries";
import { renderServerGroup } from "./render-server-group";
import type { CsvImportTarget, ObjectKind, PropsModalState, SidebarRenderCtx } from "./types";

export function ServerSidebar({
  onEditConnection,
}: {
  onEditConnection?: (projectId: string) => void;
}) {
  const projects = useProjectStore((s) => s.projects);
  const status = useProjectStore((s) => s.status);
  const serverDatabases = useProjectStore((s) => s.serverDatabases);
  const serverTablespaces = useProjectStore((s) => s.serverTablespaces);
  const schemas = useProjectStore((s) => s.schemas);
  const tables = useProjectStore((s) => s.tables);
  const columnDetails = useProjectStore((s) => s.columnDetails);
  const indexes = useProjectStore((s) => s.indexes);
  const constraints = useProjectStore((s) => s.constraints);
  const triggers = useProjectStore((s) => s.triggers);
  const rules = useProjectStore((s) => s.rules);
  const policies = useProjectStore((s) => s.policies);
  const views = useProjectStore((s) => s.views);
  const materializedViews = useProjectStore((s) => s.materializedViews);
  const functions = useProjectStore((s) => s.functions);
  const triggerFunctions = useProjectStore((s) => s.triggerFunctions);
  const connect = useProjectStore((s) => s.connect);
  const loadTables = useProjectStore((s) => s.loadTables);
  const loadColumns = useProjectStore((s) => s.loadColumns);
  const loadTableMetadata = useProjectStore((s) => s.loadTableMetadata);
  const loadSchemaObjects = useProjectStore((s) => s.loadSchemaObjects);
  const refreshConnection = useProjectStore((s) => s.refreshConnection);
  const deleteProject = useProjectStore((s) => s.deleteProject);
  const addDatabaseToServer = useProjectStore((s) => s.addDatabaseToServer);
  const renameServer = useProjectStore((s) => s.renameServer);
  const setConnectionModalOpen = useUIStore((s) => s.setConnectionModalOpen);
  const openTab = useTabStore((s) => s.openTab);
  const openMonitorTab = useTabStore((s) => s.openMonitorTab);
  const openERDTab = useTabStore((s) => s.openERDTab);
  const openNotifyTab = useTabStore((s) => s.openNotifyTab);
  const openRolesTab = useTabStore((s) => s.openRolesTab);
  const openSchemaDiffTab = useTabStore((s) => s.openSchemaDiffTab);
  const openExtensionsTab = useTabStore((s) => s.openExtensionsTab);
  const openEnumsTab = useTabStore((s) => s.openEnumsTab);
  const openPgSettingsTab = useTabStore((s) => s.openPgSettingsTab);
  const savedQueries = useQueryStore((s) => s.queries);
  const loadQueries = useQueryStore((s) => s.loadQueries);
  const queriesLoaded = useQueryStore((s) => s.loaded);
  const removeQuery = useQueryStore((s) => s.removeQuery);
  const { menu, showMenu, closeMenu } = useContextMenu();

  const [propsModal, setPropsModal] = React.useState<PropsModalState>({
    open: false,
    objectType: "table",
    projectId: "",
    schema: "",
    name: "",
  });

  const openProperties = (
    objectType: ObjectKind,
    projectId: string,
    schema: string,
    name: string,
  ) => {
    setPropsModal({ open: true, objectType, projectId, schema, name });
  };

  const [csvImportTarget, setCsvImportTarget] = React.useState<CsvImportTarget | null>(null);

  React.useEffect(() => {
    if (!queriesLoaded) void loadQueries();
  }, [queriesLoaded, loadQueries]);

  const [addDbSource, setAddDbSource] = React.useState<string | null>(null);
  const [renameServerPids, setRenameServerPids] = React.useState<string[] | null>(null);
  const renameTarget = renameServerPids?.[0] ? projects[renameServerPids[0]] : undefined;
  const [expanded, setExpanded] = React.useState<Record<string, boolean>>({});
  const [loading, setLoading] = React.useState<Record<string, boolean>>({});
  const [selectedItem, setSelectedItem] = React.useState<string | null>(null);

  const toggle = (key: string) => setExpanded((p) => ({ ...p, [key]: !p[key] }));
  const isOpen = (key: string, defaultOpen = false) => expanded[key] ?? defaultOpen;

  const setLoad = (key: string, v: boolean) => setLoading((p) => ({ ...p, [key]: v }));

  const onConnect = async (projectId: string) => {
    setLoad(projectId, true);
    await connect(projectId);
    setLoad(projectId, false);
  };

  const onExpandSchema = async (projectId: string, schema: string) => {
    const key = `schema::${projectId}::${schema}`;
    toggle(key);
    if (!isOpen(key)) {
      const tKey = `${projectId}::${schema}`;
      if (!tables[tKey]) {
        setLoad(key, true);
        try {
          await Promise.all([loadTables(projectId, schema), loadSchemaObjects(projectId, schema)]);
        } catch (e) {
          console.error("Failed to load schema objects:", e);
        } finally {
          setLoad(key, false);
        }
      }
    }
  };

  const onExpandTable = async (projectId: string, schema: string, table: string) => {
    const key = `table::${projectId}::${schema}::${table}`;
    toggle(key);
    const metaKey = `${projectId}::${schema}::${table}`;
    if (!isOpen(key) && !columnDetails[metaKey]) {
      setLoad(key, true);
      try {
        await loadTableMetadata(projectId, schema, table);
      } catch (e) {
        console.error("Failed to load table metadata:", e);
      } finally {
        setLoad(key, false);
      }
    }
  };

  const onOpenTableQuery = (projectId: string, schema: string, table: string) => {
    openTab(projectId, `SELECT * FROM "${schema}"."${table}" LIMIT 100;`);
  };

  const copy = (text: string) => navigator.clipboard.writeText(text);

  const ctx: SidebarRenderCtx = {
    projects,
    status,
    serverDatabases,
    serverTablespaces,
    schemas,
    tables,
    columnDetails,
    indexes,
    constraints,
    triggers,
    rules,
    policies,
    views,
    materializedViews,
    functions,
    triggerFunctions,
    connect,
    loadColumns,
    refreshConnection,
    deleteProject,
    addDatabaseToServer,
    openTab,
    openMonitorTab,
    openERDTab,
    openNotifyTab,
    openRolesTab,
    openSchemaDiffTab,
    openExtensionsTab,
    openEnumsTab,
    openPgSettingsTab,
    loading,
    selectedItem,
    setSelectedItem,
    setCsvImportTarget,
    setAddDbSource,
    setRenameServerPids,
    openProperties,
    toggle,
    isOpen,
    onConnect,
    onExpandSchema,
    onExpandTable,
    onOpenTableQuery,
    copy,
    showMenu,
    onEditConnection,
  };

  return (
    <div className="flex h-full flex-col border-r border-sidebar-border bg-sidebar select-none">
      <div className="flex h-12 items-center justify-between border-b border-sidebar-border px-3">
        <span className="tracking-widest uppercase text-[10px] font-semibold text-sidebar-foreground">
          CONNECTIONS
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6"
          onClick={() => setConnectionModalOpen(true)}
        >
          <Plus className="h-3 w-3" />
        </Button>
      </div>
      <div className="flex-1 overflow-y-auto overflow-x-auto p-1">
        {(() => {
          const entries = Object.entries(projects);
          // Auto-group by server fingerprint (host:port:user:ssh)
          const serverFp = (d: ProjectDetails) =>
            `${d.host}\0${d.port}\0${d.username}\0${d.sshEnabled === "true" ? `${d.sshHost}:${d.sshPort}` : ""}`;
          const serverGroups = new Map<string, string[]>();
          for (const [pid, d] of entries) {
            const fp = serverFp(d);
            if (!serverGroups.has(fp)) serverGroups.set(fp, []);
            serverGroups.get(fp)?.push(pid);
          }

          return (
            <>
              {Array.from(serverGroups.entries()).map(([fp, pids]) =>
                renderServerGroup(ctx, fp, pids),
              )}
            </>
          );
        })()}
      </div>

      {renderSavedQueries(ctx, savedQueries, removeQuery)}

      <AddDatabaseDialog
        open={!!addDbSource}
        onOpenChange={(open) => {
          if (!open) setAddDbSource(null);
        }}
        sourceProjectId={addDbSource ?? ""}
        projects={projects}
        onAdd={async (name, database) => {
          if (addDbSource) {
            await addDatabaseToServer(addDbSource, name, database);
            setAddDbSource(null);
          }
        }}
      />

      <RenameServerDialog
        open={!!renameServerPids}
        onOpenChange={(open) => {
          if (!open) setRenameServerPids(null);
        }}
        address={renameTarget ? `${renameTarget.host}:${renameTarget.port}` : ""}
        currentAlias={renameServerPids ? (serverGroupAlias(projects, renameServerPids) ?? "") : ""}
        onRename={async (alias) => {
          if (renameServerPids) {
            await renameServer(renameServerPids, alias);
            setRenameServerPids(null);
          }
        }}
      />

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={closeMenu} />}
      <ObjectPropertiesModal
        open={propsModal.open}
        onOpenChange={(open) => setPropsModal((p) => ({ ...p, open }))}
        objectType={propsModal.objectType}
        projectId={propsModal.projectId}
        schema={propsModal.schema}
        name={propsModal.name}
      />
      {csvImportTarget && (
        <CSVImportModal
          open={!!csvImportTarget}
          onOpenChange={(open) => {
            if (!open) setCsvImportTarget(null);
          }}
          projectId={csvImportTarget.projectId}
          schema={csvImportTarget.schema}
          table={csvImportTarget.table}
          tableColumns={csvImportTarget.columns}
        />
      )}
    </div>
  );
}
