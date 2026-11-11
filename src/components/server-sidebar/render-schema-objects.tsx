import {
  Copy,
  Eye,
  FileCode,
  FileUp,
  FolderOpen,
  Layers,
  Plus,
  RefreshCw,
  Settings2,
  Table,
  Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { ProjectConnectionStatus } from "@/types";
import { I } from "./constants";
import { ddlFunctionQuery, ddlTableQuery, ddlViewQuery } from "./ddl-queries";
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
    views,
    materializedViews,
    functions,
    triggerFunctions,
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
    const schemaStoreKey = `${pid}::${schema}`;
    const schemaTables = tables[schemaStoreKey];
    const schemaViews = views[schemaStoreKey];
    const schemaMatViews = materializedViews[schemaStoreKey];
    const schemaFns = functions[schemaStoreKey];
    const schemaTrigFns = triggerFunctions[schemaStoreKey];
    const isSchemaOpen = isOpen(sKey);

    return (
      <div key={schema}>
        <TreeRow
          indent={I.schema}
          icon={<FolderOpen className="h-3.5 w-3.5 text-muted-foreground" />}
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
            {/* Tables category */}
            <SectionHeader
              indent={I.schemaObj}
              label={`Tables${schemaTables ? ` (${schemaTables.length})` : ""}`}
              icon={<Table className="h-3 w-3" />}
              sectionKey={`${sKey}::tables`}
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
                      indent={I.table}
                      icon={<Table className="h-3.5 w-3.5 text-muted-foreground" />}
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
                        <span className="inline-block w-16 shrink-0 rounded-full bg-accent/60 pr-1 py-0.5 text-right font-mono text-[10px] tabular-nums text-muted-foreground">
                          {ti.size}
                        </span>
                      }
                    />
                    {isTableOpen && renderTableDetails(ctx, pid, schema, ti.name)}
                  </div>
                );
              })}

            {/* Views category */}
            {schemaViews && schemaViews.length > 0 && (
              <>
                <SectionHeader
                  indent={I.schemaObj}
                  label={`Views (${schemaViews.length})`}
                  icon={<Eye className="h-3 w-3" />}
                  sectionKey={`${sKey}::views`}
                  expanded={isOpen(`${sKey}::views`)}
                  onClick={() => toggle(`${sKey}::views`)}
                />
                {isOpen(`${sKey}::views`) &&
                  schemaViews.map((v) => {
                    const vKey = `view::${pid}::${schema}::${v}`;
                    return (
                      <TreeRow
                        key={v}
                        indent={I.table}
                        icon={<Eye className="h-3.5 w-3.5 text-muted-foreground" />}
                        label={v}
                        selected={selectedItem === vKey}
                        onClick={() => {
                          setSelectedItem(vKey);
                          onOpenTableQuery(pid, schema, v);
                        }}
                        onContextMenu={(e) => {
                          setSelectedItem(vKey);
                          showMenu(e, [
                            {
                              label: "SELECT TOP 100",
                              icon: <Eye className="h-3 w-3" />,
                              onClick: () => onOpenTableQuery(pid, schema, v),
                            },
                            { separator: true as const },
                            {
                              label: "Properties",
                              icon: <Settings2 className="h-3 w-3" />,
                              onClick: () => openProperties("view", pid, schema, v),
                            },
                            {
                              label: "Show CREATE VIEW",
                              icon: <FileCode className="h-3 w-3" />,
                              onClick: () => openTab(pid, ddlViewQuery(schema, v)),
                            },
                            { separator: true as const },
                            {
                              label: "Copy Name",
                              icon: <Copy className="h-3 w-3" />,
                              onClick: () => copy(`"${schema}"."${v}"`),
                            },
                          ]);
                        }}
                      />
                    );
                  })}
              </>
            )}

            {/* Materialized Views category */}
            {schemaMatViews && schemaMatViews.length > 0 && (
              <>
                <SectionHeader
                  indent={I.schemaObj}
                  label={`Materialized Views (${schemaMatViews.length})`}
                  icon={<Layers className="h-3 w-3" />}
                  sectionKey={`${sKey}::matviews`}
                  expanded={isOpen(`${sKey}::matviews`)}
                  onClick={() => toggle(`${sKey}::matviews`)}
                />
                {isOpen(`${sKey}::matviews`) &&
                  schemaMatViews.map((mv) => {
                    const mvKey = `matview::${pid}::${schema}::${mv}`;
                    return (
                      <TreeRow
                        key={mv}
                        indent={I.table}
                        icon={<Layers className="h-3.5 w-3.5 text-muted-foreground" />}
                        label={mv}
                        selected={selectedItem === mvKey}
                        onClick={() => {
                          setSelectedItem(mvKey);
                          onOpenTableQuery(pid, schema, mv);
                        }}
                        onContextMenu={(e) => {
                          setSelectedItem(mvKey);
                          showMenu(e, [
                            {
                              label: "SELECT TOP 100",
                              icon: <Layers className="h-3 w-3" />,
                              onClick: () => onOpenTableQuery(pid, schema, mv),
                            },
                            {
                              label: "REFRESH",
                              icon: <RefreshCw className="h-3 w-3" />,
                              onClick: () =>
                                openTab(pid, `REFRESH MATERIALIZED VIEW "${schema}"."${mv}";`),
                            },
                            { separator: true as const },
                            {
                              label: "Properties",
                              icon: <Settings2 className="h-3 w-3" />,
                              onClick: () => openProperties("matview", pid, schema, mv),
                            },
                            { separator: true as const },
                            {
                              label: "Copy Name",
                              icon: <Copy className="h-3 w-3" />,
                              onClick: () => copy(`"${schema}"."${mv}"`),
                            },
                          ]);
                        }}
                      />
                    );
                  })}
              </>
            )}

            {/* Functions category */}
            {schemaFns && schemaFns.length > 0 && (
              <>
                <SectionHeader
                  indent={I.schemaObj}
                  label={`Functions (${schemaFns.length})`}
                  icon={<FileCode className="h-3 w-3" />}
                  sectionKey={`${sKey}::fns`}
                  expanded={isOpen(`${sKey}::fns`)}
                  onClick={() => toggle(`${sKey}::fns`)}
                />
                {isOpen(`${sKey}::fns`) &&
                  schemaFns.map((fn, i) => {
                    const fnKey = `fn::${pid}::${schema}::${fn.name}::${i}`;
                    return (
                      <div
                        key={`${fn.name}-${i}`}
                        className={cn(
                          "relative flex items-center gap-1.5 py-0.5 rounded-sm whitespace-nowrap select-none",
                          selectedItem === fnKey ? "bg-primary/10" : "hover:bg-sidebar-accent",
                        )}
                        style={{ paddingLeft: `${I.table}px` }}
                        onClick={() => setSelectedItem(fnKey)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          setSelectedItem(fnKey);
                          showMenu(e, [
                            {
                              label: "Show Definition",
                              icon: <FileCode className="h-3 w-3" />,
                              onClick: () => openTab(pid, ddlFunctionQuery(schema, fn.name)),
                            },
                            {
                              label: "Properties",
                              icon: <Settings2 className="h-3 w-3" />,
                              onClick: () => openProperties("function", pid, schema, fn.name),
                            },
                            { separator: true as const },
                            {
                              label: "Copy Name",
                              icon: <Copy className="h-3 w-3" />,
                              onClick: () => copy(fn.name),
                            },
                          ]);
                        }}
                      >
                        <FileCode className="h-3 w-3 shrink-0 text-muted-foreground/50" />
                        <span className="font-mono text-[11px] text-foreground">
                          {fn.name}({fn.arguments ? "..." : ""})
                        </span>
                        <span className="font-mono text-[10px] text-muted-foreground">
                          {fn.returnType}
                        </span>
                      </div>
                    );
                  })}
              </>
            )}

            {/* Trigger Functions category */}
            {schemaTrigFns && schemaTrigFns.length > 0 && (
              <>
                <SectionHeader
                  indent={I.schemaObj}
                  label={`Trigger Functions (${schemaTrigFns.length})`}
                  icon={<Zap className="h-3 w-3" />}
                  sectionKey={`${sKey}::trigfns`}
                  expanded={isOpen(`${sKey}::trigfns`)}
                  onClick={() => toggle(`${sKey}::trigfns`)}
                />
                {isOpen(`${sKey}::trigfns`) &&
                  schemaTrigFns.map((fn, i) => {
                    const tfKey = `trigfn::${pid}::${schema}::${fn.name}::${i}`;
                    return (
                      <div
                        key={`${fn.name}-${i}`}
                        className={cn(
                          "relative flex items-center gap-1.5 py-0.5 rounded-sm whitespace-nowrap select-none",
                          selectedItem === tfKey ? "bg-primary/10" : "hover:bg-sidebar-accent",
                        )}
                        style={{ paddingLeft: `${I.table}px` }}
                        onClick={() => setSelectedItem(tfKey)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          setSelectedItem(tfKey);
                          showMenu(e, [
                            {
                              label: "Show Definition",
                              icon: <FileCode className="h-3 w-3" />,
                              onClick: () => openTab(pid, ddlFunctionQuery(schema, fn.name)),
                            },
                            {
                              label: "Properties",
                              icon: <Settings2 className="h-3 w-3" />,
                              onClick: () =>
                                openProperties("trigger-function", pid, schema, fn.name),
                            },
                            { separator: true as const },
                            {
                              label: "Copy Name",
                              icon: <Copy className="h-3 w-3" />,
                              onClick: () => copy(fn.name),
                            },
                          ]);
                        }}
                      >
                        <Zap className="h-3 w-3 shrink-0 text-muted-foreground/50" />
                        <span className="font-mono text-[11px] text-foreground">{fn.name}()</span>
                        <span className="font-mono text-[10px] text-muted-foreground">trigger</span>
                      </div>
                    );
                  })}
              </>
            )}
          </>
        )}
      </div>
    );
  });
}
