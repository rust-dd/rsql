import { Columns3, Copy, Key, Link2, Lock, ScrollText, Shield, Zap } from "lucide-react";
import { DEPTH, LEAF_ICON, SECTION_ICON } from "./constants";
import { SectionHeader } from "./section-header";
import { TreeRow } from "./tree-row";
import type { SidebarRenderCtx } from "./types";

export function renderTableDetails(
  ctx: SidebarRenderCtx,
  pid: string,
  schema: string,
  tableName: string,
) {
  const {
    columnDetails,
    indexes,
    constraints,
    triggers,
    rules,
    policies,
    isOpen,
    toggle,
    showMenu,
    copy,
  } = ctx;
  const tKey = `table::${pid}::${schema}::${tableName}`;
  const metaKey = `${pid}::${schema}::${tableName}`;
  const cols = columnDetails[metaKey];
  const idxs = indexes[metaKey];
  const cons = constraints[metaKey];
  const trigs = triggers[metaKey];
  const rls = rules[metaKey];
  const pols = policies[metaKey];
  const pkCols = new Set((idxs ?? []).filter((i) => i.isPrimary).map((i) => i.columnName));
  const indexNames = Array.from(new Set((idxs ?? []).map((i) => i.indexName)));
  const constraintNames = Array.from(new Set((cons ?? []).map((c) => c.constraintName)));

  if (!cols) return null;

  return (
    <>
      <SectionHeader
        depth={DEPTH.section}
        label="Columns"
        count={cols.length}
        icon={<Columns3 className={SECTION_ICON} />}
        expanded={isOpen(`${tKey}::cols`, true)}
        onClick={() => toggle(`${tKey}::cols`)}
      />
      {isOpen(`${tKey}::cols`, true) &&
        cols.map((c) => (
          <TreeRow
            key={c.name}
            depth={DEPTH.item}
            icon={
              pkCols.has(c.name) ? (
                <Key className="h-3 w-3 text-warning" />
              ) : (
                <Columns3 className={LEAF_ICON} />
              )
            }
            label={c.name}
            detail={
              <>
                {c.dataType}
                {c.nullable && <span className="ml-1 text-muted-foreground/50">NULL</span>}
              </>
            }
            onContextMenu={(e) =>
              showMenu(e, [
                {
                  label: "Copy Column Name",
                  icon: <Copy className="h-3 w-3" />,
                  onClick: () => copy(c.name),
                },
              ])
            }
          />
        ))}

      {idxs && idxs.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.section}
            label="Indexes"
            count={indexNames.length}
            icon={<Key className={SECTION_ICON} />}
            expanded={isOpen(`${tKey}::idx`)}
            onClick={() => toggle(`${tKey}::idx`)}
          />
          {isOpen(`${tKey}::idx`) &&
            indexNames.map((name) => {
              const idxEntries = idxs.filter((i) => i.indexName === name);
              const f = idxEntries[0];
              return (
                <TreeRow
                  key={name}
                  depth={DEPTH.item}
                  icon={
                    f.isPrimary ? (
                      <Key className="h-3 w-3 text-warning" />
                    ) : f.isUnique ? (
                      <Shield className="h-3 w-3 text-blue-500" />
                    ) : (
                      <Key className={LEAF_ICON} />
                    )
                  }
                  label={name}
                  detail={
                    <>
                      ({idxEntries.map((e) => e.columnName).join(", ")})
                      {f.isUnique && <span className="ml-1 text-blue-500/60">UNIQUE</span>}
                    </>
                  }
                />
              );
            })}
        </>
      )}

      {cons && cons.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.section}
            label="Constraints"
            count={constraintNames.length}
            icon={<Link2 className={SECTION_ICON} />}
            expanded={isOpen(`${tKey}::con`)}
            onClick={() => toggle(`${tKey}::con`)}
          />
          {isOpen(`${tKey}::con`) &&
            constraintNames.map((name) => (
              <TreeRow
                key={name}
                depth={DEPTH.item}
                icon={<Link2 className={LEAF_ICON} />}
                label={name}
                detail={cons.find((c) => c.constraintName === name)?.constraintType}
              />
            ))}
        </>
      )}

      {trigs && trigs.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.section}
            label="Triggers"
            count={trigs.length}
            icon={<Zap className={SECTION_ICON} />}
            expanded={isOpen(`${tKey}::trig`)}
            onClick={() => toggle(`${tKey}::trig`)}
          />
          {isOpen(`${tKey}::trig`) &&
            trigs.map((t) => (
              <TreeRow
                key={`${t.triggerName}-${t.event}`}
                depth={DEPTH.item}
                icon={<Zap className={LEAF_ICON} />}
                label={t.triggerName}
                detail={`${t.timing} ${t.event}`}
              />
            ))}
        </>
      )}

      {rls && rls.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.section}
            label="Rules"
            count={rls.length}
            icon={<ScrollText className={SECTION_ICON} />}
            expanded={isOpen(`${tKey}::rules`)}
            onClick={() => toggle(`${tKey}::rules`)}
          />
          {isOpen(`${tKey}::rules`) &&
            rls.map((r) => (
              <TreeRow
                key={r.ruleName}
                depth={DEPTH.item}
                icon={<ScrollText className={LEAF_ICON} />}
                label={r.ruleName}
                detail={r.event}
              />
            ))}
        </>
      )}

      {pols && pols.length > 0 && (
        <>
          <SectionHeader
            depth={DEPTH.section}
            label="RLS Policies"
            count={pols.length}
            icon={<Lock className={SECTION_ICON} />}
            expanded={isOpen(`${tKey}::pol`)}
            onClick={() => toggle(`${tKey}::pol`)}
          />
          {isOpen(`${tKey}::pol`) &&
            pols.map((p) => (
              <TreeRow
                key={p.policyName}
                depth={DEPTH.item}
                icon={<Lock className={LEAF_ICON} />}
                label={p.policyName}
                detail={`${p.permissive} ${p.command}`}
              />
            ))}
        </>
      )}
    </>
  );
}
