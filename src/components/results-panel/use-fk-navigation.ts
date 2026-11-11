import { useCallback, useEffect, useState } from "react";
import { DriverFactory, type ForeignKey } from "@/lib/database-driver";
import { executeTabQuery } from "@/lib/query-execution";
import { quoteIdent, quoteLiteral } from "@/lib/sql-utils";
import { useProjectStore } from "@/stores/project-store";
import { useTabStore } from "@/stores/tab-store";

export interface FkTarget {
  schema: string;
  table: string;
  column: string;
}

export function fkTargets(fks: ForeignKey[], table: string): Map<string, FkTarget> {
  const targets = new Map<string, FkTarget>();
  for (const fk of fks) {
    // The backend lists single-column keys first, so the first one is the most specific.
    if (fk.sourceTable !== table || targets.has(fk.sourceColumn)) continue;
    targets.set(fk.sourceColumn, {
      schema: fk.targetSchema,
      table: fk.targetTable,
      column: fk.targetColumn,
    });
  }
  return targets;
}

interface UseFkNavigationArgs {
  projectId: string | undefined;
  editableTable: { schema: string; table: string } | null;
}

export function useFkNavigation({ projectId, editableTable }: UseFkNavigationArgs) {
  const [fkMap, setFkMap] = useState<Map<string, FkTarget>>(new Map());
  const schema = editableTable?.schema;
  const table = editableTable?.table;

  useEffect(() => {
    if (!schema || !table || !projectId) {
      setFkMap(new Map());
      return;
    }
    const project = useProjectStore.getState().projects[projectId];
    if (!project) return;

    let stale = false;
    DriverFactory.getDriver(project.driver)
      .loadForeignKeys(projectId, schema)
      .then((fks) => {
        if (!stale) setFkMap(fkTargets(fks, table));
      })
      .catch(() => {
        if (!stale) setFkMap(new Map());
      });
    return () => {
      stale = true;
    };
  }, [projectId, schema, table]);

  const handleFKNavigate = useCallback(
    (colName: string, value: string) => {
      const target = fkMap.get(colName);
      if (!target || !projectId) return;
      const sql = `SELECT * FROM ${quoteIdent(target.schema)}.${quoteIdent(target.table)} WHERE ${quoteIdent(target.column)} = ${quoteLiteral(value)} LIMIT 100`;
      useTabStore.getState().openTab(projectId, sql);
      void executeTabQuery();
    },
    [fkMap, projectId],
  );

  return { fkMap, handleFKNavigate };
}
