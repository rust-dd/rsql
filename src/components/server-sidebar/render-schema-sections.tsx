import { Copy, Eye, FileCode, Layers, RefreshCw, Settings2, Zap } from "lucide-react";
import { DEPTH, SECTION_ICON } from "./constants";
import { ddlFunctionQuery, ddlViewQuery } from "./ddl-queries";
import { SectionHeader } from "./section-header";
import { TreeRow } from "./tree-row";
import type { SidebarRenderCtx } from "./types";

/** Views, materialized views, functions and trigger functions of one schema. */
export function renderSchemaSections(ctx: SidebarRenderCtx, pid: string, schema: string) {
  const {
    views,
    materializedViews,
    functions,
    triggerFunctions,
    selectedItem,
    setSelectedItem,
    openProperties,
    isOpen,
    toggle,
    onOpenTableQuery,
    openTab,
    showMenu,
    copy,
  } = ctx;

  const sKey = `schema::${pid}::${schema}`;
  const storeKey = `${pid}::${schema}`;
  const schemaViews = views[storeKey];
  const schemaMatViews = materializedViews[storeKey];
  const schemaFns = functions[storeKey];
  const schemaTrigFns = triggerFunctions[storeKey];

  return (
    <>
      {schemaViews && schemaViews.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.schemaObj}
            label="Views"
            count={schemaViews.length}
            icon={<Eye className={SECTION_ICON} />}
            expanded={isOpen(`${sKey}::views`)}
            onClick={() => toggle(`${sKey}::views`)}
          />
          {isOpen(`${sKey}::views`) &&
            schemaViews.map((v) => {
              const vKey = `view::${pid}::${schema}::${v}`;
              return (
                <TreeRow
                  key={v}
                  depth={DEPTH.table}
                  icon={<Eye className={SECTION_ICON} />}
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

      {schemaMatViews && schemaMatViews.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.schemaObj}
            label="Materialized Views"
            count={schemaMatViews.length}
            icon={<Layers className={SECTION_ICON} />}
            expanded={isOpen(`${sKey}::matviews`)}
            onClick={() => toggle(`${sKey}::matviews`)}
          />
          {isOpen(`${sKey}::matviews`) &&
            schemaMatViews.map((mv) => {
              const mvKey = `matview::${pid}::${schema}::${mv}`;
              return (
                <TreeRow
                  key={mv}
                  depth={DEPTH.table}
                  icon={<Layers className={SECTION_ICON} />}
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

      {schemaFns && schemaFns.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.schemaObj}
            label="Functions"
            count={schemaFns.length}
            icon={<FileCode className={SECTION_ICON} />}
            expanded={isOpen(`${sKey}::fns`)}
            onClick={() => toggle(`${sKey}::fns`)}
          />
          {isOpen(`${sKey}::fns`) &&
            schemaFns.map((fn, i) => {
              const fnKey = `fn::${pid}::${schema}::${fn.name}::${i}`;
              return (
                <TreeRow
                  key={`${fn.name}-${i}`}
                  depth={DEPTH.table}
                  icon={<FileCode className={SECTION_ICON} />}
                  label={`${fn.name}(${fn.arguments ? "..." : ""})`}
                  detail={fn.returnType}
                  selected={selectedItem === fnKey}
                  onClick={() => setSelectedItem(fnKey)}
                  onContextMenu={(e) => {
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
                />
              );
            })}
        </>
      )}

      {schemaTrigFns && schemaTrigFns.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.schemaObj}
            label="Trigger Functions"
            count={schemaTrigFns.length}
            icon={<Zap className={SECTION_ICON} />}
            expanded={isOpen(`${sKey}::trigfns`)}
            onClick={() => toggle(`${sKey}::trigfns`)}
          />
          {isOpen(`${sKey}::trigfns`) &&
            schemaTrigFns.map((fn, i) => {
              const tfKey = `trigfn::${pid}::${schema}::${fn.name}::${i}`;
              return (
                <TreeRow
                  key={`${fn.name}-${i}`}
                  depth={DEPTH.table}
                  icon={<Zap className={SECTION_ICON} />}
                  label={`${fn.name}()`}
                  detail="trigger"
                  selected={selectedItem === tfKey}
                  onClick={() => setSelectedItem(tfKey)}
                  onContextMenu={(e) => {
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
                        onClick: () => openProperties("trigger-function", pid, schema, fn.name),
                      },
                      { separator: true as const },
                      {
                        label: "Copy Name",
                        icon: <Copy className="h-3 w-3" />,
                        onClick: () => copy(fn.name),
                      },
                    ]);
                  }}
                />
              );
            })}
        </>
      )}
    </>
  );
}
