import { Copy, FileCode, FileUp, FolderOpen, Layers, Plus, Settings2, Table } from "lucide-react";
import { ProjectConnectionStatus } from "@/types";
import { DEPTH, SECTION_ICON } from "./constants";
import { ddlTableQuery } from "./ddl-queries";
import { renderSchemaSections } from "./render-schema-sections";
import { renderTableDetails } from "./render-table-details";
import { SectionHeader } from "./section-header";
import { TreeRow } from "./tree-row";
import type { SidebarRenderCtx } from "./types";

/** Render schemas + tables/views/functions for a connected database project */
export function renderSchemas(ctx: SidebarRenderCtx, pid: string) {
  const {
    schemas,
    status,
    tables,
    loading,
    selectedItem,
    setSelectedItem,
    setCsvImportTarget,
    openProperties,
    isOpen,
    toggle,
    onExpandSchema,
    onExpandTable,
    onOpenTableQuery,
    openTab,
    openERDTab,
    loadColumns,
    showMenu,
    copy,
  } = ctx;

  const projectSchemas = schemas[pid] || [];
  const isConnected = status[pid] === ProjectConnectionStatus.Connected;
  if (!isConnected || !projectSchemas.length) return null;

  return projectSchemas.map((schema) => {
    const sKey = `schema::${pid}::${schema}`;
    const schemaTables = tables[`${pid}::${schema}`];
    const isSchemaOpen = isOpen(sKey);

    return (
      <div key={schema}>
        <TreeRow
          depth={DEPTH.schema}
          icon={<FolderOpen className={SECTION_ICON} />}
          label={schema}
          expanded={isSchemaOpen}
          loading={loading[sKey]}
          onClick={() => void onExpandSchema(pid, schema)}
          onContextMenu={(e) =>
            showMenu(e, [
              {
                label: "ERD Diagram",
                icon: <Layers className="h-3 w-3" />,
                onClick: () => openERDTab(pid, schema),
              },
              {
                label: "Copy Schema Name",
                icon: <Copy className="h-3 w-3" />,
                onClick: () => copy(schema),
              },
              {
                label: "New Query",
                icon: <Plus className="h-3 w-3" />,
                onClick: () => openTab(pid, `-- Schema: ${schema}\n`),
              },
            ])
          }
        />

        {isSchemaOpen && (
          <>
            <SectionHeader
              depth={DEPTH.schemaObj}
              label="Tables"
              count={schemaTables?.length}
              icon={<Table className={SECTION_ICON} />}
              expanded={isOpen(`${sKey}::tables`, true)}
              onClick={() => toggle(`${sKey}::tables`)}
            />
            {isOpen(`${sKey}::tables`, true) &&
              schemaTables?.map((ti) => {
                const tKey = `table::${pid}::${schema}::${ti.name}`;
                const isTableOpen = isOpen(tKey);

                return (
                  <div key={ti.name}>
                    <TreeRow
                      depth={DEPTH.table}
                      icon={<Table className={SECTION_ICON} />}
                      label={ti.name}
                      expanded={isTableOpen}
                      loading={loading[tKey]}
                      selected={selectedItem === tKey}
                      onClick={() => {
                        setSelectedItem(tKey);
                        void onExpandTable(pid, schema, ti.name);
                      }}
                      onDoubleClick={() => onOpenTableQuery(pid, schema, ti.name)}
                      onContextMenu={(e) => {
                        setSelectedItem(tKey);
                        showMenu(e, [
                          { header: "Query" },
                          {
                            label: "SELECT TOP 100",
                            icon: <Table className="h-3 w-3" />,
                            onClick: () => onOpenTableQuery(pid, schema, ti.name),
                          },
                          {
                            label: "SELECT COUNT(*)",
                            icon: <Table className="h-3 w-3" />,
                            onClick: () =>
                              openTab(pid, `SELECT COUNT(*) FROM "${schema}"."${ti.name}";`),
                          },
                          { separator: true as const },
                          {
                            label: "Import CSV",
                            icon: <FileUp className="h-3 w-3" />,
                            onClick: () => {
                              void loadColumns(pid, schema, ti.name).then((cols) => {
                                setCsvImportTarget({
                                  projectId: pid,
                                  schema,
                                  table: ti.name,
                                  columns: cols,
                                });
                              });
                            },
                          },
                          { separator: true as const },
                          {
                            label: "Properties",
                            icon: <Settings2 className="h-3 w-3" />,
                            onClick: () => openProperties("table", pid, schema, ti.name),
                          },
                          {
                            label: "Show CREATE TABLE",
                            icon: <FileCode className="h-3 w-3" />,
                            onClick: () => openTab(pid, ddlTableQuery(schema, ti.name)),
                          },
                          { separator: true as const },
                          {
                            label: "Copy Name",
                            icon: <Copy className="h-3 w-3" />,
                            onClick: () => copy(`"${schema}"."${ti.name}"`),
                            shortcut: navigator.platform.includes("Mac") ? "⌘C" : "Ctrl+C",
                          },
                        ]);
                      }}
                      trailing={
                        <span className="font-mono text-[10px] tabular-nums text-muted-foreground/80">
                          {ti.size}
                        </span>
                      }
                    />
                    {isTableOpen && renderTableDetails(ctx, pid, schema, ti.name)}
                  </div>
                );
              })}

            {renderSchemaSections(ctx, pid, schema)}
          </>
        )}
      </div>
    );
  });
}
