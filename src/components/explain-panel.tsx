import { ArrowRight, ChevronDown, ChevronRight, Clock, Layers, Timer } from "lucide-react";
import React, { useMemo } from "react";
import { cn } from "@/lib/utils";
import type { ExplainNode, ExplainPlan } from "@/types";

interface ExplainPanelProps {
  plan: ExplainPlan;
}

/** Without ANALYZE a plan has estimates only, so it must not show actual times or rows. */
function isAnalyzed(plan: ExplainPlan): boolean {
  return plan.Plan["Actual Rows"] != null;
}

export function ExplainPanel({ plan }: ExplainPanelProps) {
  const root = plan.Plan;
  const analyzed = isAnalyzed(plan);
  const maxWeight = analyzed ? (root["Actual Total Time"] ?? 0) : root["Total Cost"];

  return (
    <div className="flex h-full flex-col overflow-auto bg-card">
      {/* Summary bar */}
      <div className="flex items-center gap-4 border-b px-4 py-2 flex-shrink-0">
        {!analyzed && (
          <span className="rounded border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
            Estimated plan · not executed
          </span>
        )}
        {plan["Planning Time"] != null && (
          <div className="flex items-center gap-1.5 font-mono text-xs">
            <Timer className="h-3.5 w-3.5 text-primary" />
            <span className="text-muted-foreground">Planning:</span>
            <span className="font-medium">{plan["Planning Time"].toFixed(2)}ms</span>
          </div>
        )}
        {analyzed && (
          <div className="flex items-center gap-1.5 font-mono text-xs">
            <Clock className="h-3.5 w-3.5 text-primary" />
            <span className="text-muted-foreground">Execution:</span>
            <span className="font-medium">{plan["Execution Time"]?.toFixed(2) ?? "N/A"}ms</span>
          </div>
        )}
        <div className="flex items-center gap-1.5 font-mono text-xs">
          <Layers className="h-3.5 w-3.5 text-primary" />
          <span className="text-muted-foreground">Total Cost:</span>
          <span className="font-medium">{root["Total Cost"]?.toFixed(2)}</span>
        </div>
      </div>

      {/* Plan tree */}
      <div className="flex-1 overflow-auto p-4">
        <PlanNode node={root} depth={0} maxWeight={maxWeight} analyzed={analyzed} />
      </div>
    </div>
  );
}

interface PlanNodeProps {
  node: ExplainNode;
  depth: number;
  maxWeight: number;
  analyzed: boolean;
}

function PlanNode({ node, depth, maxWeight, analyzed }: PlanNodeProps) {
  const [expanded, setExpanded] = React.useState(true);
  const hasChildren = node.Plans && node.Plans.length > 0;

  const actualTime = node["Actual Total Time"] ?? 0;
  const actualRows = node["Actual Rows"] ?? 0;
  const planRows = node["Plan Rows"] ?? 0;
  const loops = node["Actual Loops"] ?? 1;
  const totalActualTime = actualTime * loops;

  const weight = analyzed ? totalActualTime : (node["Total Cost"] ?? 0);
  const timePercent = maxWeight > 0 ? (weight / maxWeight) * 100 : 0;
  const rowEstimateRatio = planRows > 0 ? actualRows / planRows : 1;

  const nodeLabel = useMemo(() => {
    const parts = [node["Node Type"]];
    if (node["Join Type"]) parts[0] = `${node["Join Type"]} ${parts[0]}`;
    if (node.Strategy) parts.push(`(${node.Strategy})`);
    if (node["Relation Name"]) {
      const alias = node.Alias && node.Alias !== node["Relation Name"] ? ` as ${node.Alias}` : "";
      parts.push(`on ${node["Relation Name"]}${alias}`);
    }
    if (node["Index Name"]) parts.push(`using ${node["Index Name"]}`);
    return parts.join(" ");
  }, [node]);

  const details: string[] = [];
  if (node.Filter) details.push(`Filter: ${node.Filter}`);
  if (node["Index Cond"]) details.push(`Index Cond: ${node["Index Cond"]}`);
  if (node["Hash Cond"]) details.push(`Hash Cond: ${node["Hash Cond"]}`);
  if (node["Merge Cond"]) details.push(`Merge Cond: ${node["Merge Cond"]}`);
  if (node["Sort Key"]) details.push(`Sort Key: ${node["Sort Key"].join(", ")}`);

  return (
    <div style={{ marginLeft: depth > 0 ? 20 : 0 }}>
      <div
        className="group rounded-md border p-2 mb-1 hover:bg-accent/30 transition-colors cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        {/* Node header */}
        <div className="flex items-center gap-2">
          {hasChildren ? (
            expanded ? (
              <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            )
          ) : (
            <ArrowRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/30" />
          )}
          <span className="font-mono text-xs font-semibold">{nodeLabel}</span>
        </div>

        {/* Time bar */}
        <div className="mt-1.5 ml-5.5">
          <div className="h-1.5 w-full rounded-full bg-muted overflow-hidden">
            <div
              className={cn(
                "h-full rounded-full transition-all",
                timePercent > 80
                  ? "bg-destructive"
                  : timePercent > 40
                    ? "bg-warning"
                    : "bg-success",
              )}
              style={{ width: `${Math.max(1, timePercent)}%` }}
            />
          </div>
        </div>

        {/* Stats row */}
        <div className="mt-1 ml-5.5 flex flex-wrap gap-x-4 gap-y-0.5">
          {analyzed ? (
            <>
              <Stat
                label="Time"
                value={`${totalActualTime.toFixed(2)}ms`}
                warn={timePercent > 60}
              />
              <Stat label="Rows" value={actualRows.toLocaleString()} />
              <Stat
                label="Est."
                value={planRows.toLocaleString()}
                warn={rowEstimateRatio > 10 || rowEstimateRatio < 0.1}
              />
              {loops > 1 && <Stat label="Loops" value={loops.toLocaleString()} />}
            </>
          ) : (
            <Stat label="Est. rows" value={planRows.toLocaleString()} />
          )}
          <Stat
            label="Cost"
            value={`${node["Startup Cost"]?.toFixed(1)}..${node["Total Cost"]?.toFixed(1)}`}
          />
          {node["Shared Hit Blocks"] != null && (
            <Stat label="Buffers Hit" value={node["Shared Hit Blocks"].toLocaleString()} />
          )}
          {node["Shared Read Blocks"] != null && node["Shared Read Blocks"] > 0 && (
            <Stat label="Buffers Read" value={node["Shared Read Blocks"].toLocaleString()} warn />
          )}
        </div>

        {/* Conditions */}
        {details.length > 0 && (
          <div className="mt-1 ml-5.5 space-y-0.5">
            {details.map((d) => (
              <div key={d} className="font-mono text-[10px] text-muted-foreground">
                {d}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Children */}
      {expanded && hasChildren && (
        <div className="border-l border-border/50 ml-2.5">
          {node.Plans?.map((child, i) => (
            <PlanNode
              key={i}
              node={child}
              depth={depth + 1}
              maxWeight={maxWeight}
              analyzed={analyzed}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function Stat({ label, value, warn }: { label: string; value: string; warn?: boolean }) {
  return (
    <span className="font-mono text-[10px]">
      <span className="text-muted-foreground">{label}: </span>
      <span className={cn("font-medium", warn && "text-destructive")}>{value}</span>
    </span>
  );
}
