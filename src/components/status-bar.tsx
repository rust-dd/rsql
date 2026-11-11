import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import { useHistoryStore } from "@/stores/history-store";
import { projectName, useProjectStore } from "@/stores/project-store";
import { useActiveTab } from "@/stores/tab-store";
import { getSystemResourceUsage, type SystemResourceUsage } from "@/tauri";
import { ProjectConnectionStatus } from "@/types";

export function StatusBar() {
  const activeTab = useActiveTab();
  const projects = useProjectStore((s) => s.projects);
  const status = useProjectStore((s) => s.status);
  const historyCount = useHistoryStore((s) => s.entries.length);

  const projectId = activeTab?.projectId;
  const details = projectId ? projects[projectId] : undefined;
  const connStatus = projectId ? status[projectId] : undefined;
  const [resources, setResources] = useState<SystemResourceUsage | null>(null);

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;

    const load = async () => {
      // A sample walks the process tree and reads every pool's status. Skipping
      // while one is outstanding keeps a slow sample from queueing up more.
      if (inFlight || document.hidden) return;
      inFlight = true;
      try {
        const usage = await getSystemResourceUsage();
        if (!cancelled) setResources(usage);
      } catch {
        // Ignore metrics polling errors to keep footer stable.
      } finally {
        inFlight = false;
      }
    };

    void load();
    // Two seconds rather than one: this is a footer readout, and each sample
    // costs a process-tree walk plus a lock on both connection pools.
    const id = window.setInterval(() => void load(), 2000);
    // Nothing to show while the window is hidden, so stop paying for it.
    const onVisibility = () => {
      if (!document.hidden) void load();
    };
    document.addEventListener("visibilitychange", onVisibility);

    return () => {
      cancelled = true;
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  const formatMb = (mb: number): string => {
    if (mb >= 1000) return `${(mb / 1000).toFixed(2)} GB`;
    return `${mb.toLocaleString()} MB`;
  };

  const formatMbps = (mbps: number): string => {
    if (mbps >= 1000) return `${(mbps / 1000).toFixed(2)} Gbps`;
    return `${mbps.toFixed(2)} Mbps`;
  };

  const cpu = resources ? `${resources.app_cpu_percent.toFixed(1)}%` : "--";
  const rss = resources ? formatMb(resources.app_memory_rss_mb) : "--";
  const proc = resources ? `${resources.app_process_count}` : "--";
  const conn = resources
    ? `${resources.db_connections_in_use}/${resources.db_connections_open}`
    : "--";
  const connWaiting =
    resources && resources.db_connections_waiting > 0
      ? ` (${resources.db_connections_waiting} wait)`
      : "";
  const net = resources
    ? `↓ ${formatMbps(resources.network_rx_mbps)} ↑ ${formatMbps(resources.network_tx_mbps)}`
    : "--";

  return (
    <div className="flex h-7 items-center justify-between bg-card/60 backdrop-blur-sm px-3 text-[11px] font-mono text-muted-foreground">
      <div className="flex items-center gap-2">
        {projectId && details ? (
          <div
            className={cn(
              "flex items-center gap-1.5 rounded-full px-2 py-0.5",
              connStatus === ProjectConnectionStatus.Connected && "bg-success/10 text-success",
              connStatus === ProjectConnectionStatus.Connecting && "bg-warning/10 text-warning",
              connStatus === ProjectConnectionStatus.Failed && "bg-destructive/10 text-destructive",
              !connStatus && "text-muted-foreground",
            )}
          >
            <div
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                connStatus === ProjectConnectionStatus.Connected && "bg-success",
                connStatus === ProjectConnectionStatus.Connecting && "bg-warning",
                connStatus === ProjectConnectionStatus.Failed && "bg-destructive",
                !connStatus && "bg-muted",
              )}
            />
            <span>{projectName(projects, projectId)}</span>
            <span className="opacity-50">&bull;</span>
            <span>{details.database}</span>
          </div>
        ) : (
          <span>No connection</span>
        )}
      </div>

      <div className="flex items-center gap-2">
        <span>APP CPU {cpu}</span>
        <span className="opacity-40">&bull;</span>
        <span>RSS {rss}</span>
        <span className="opacity-40">&bull;</span>
        <span>PROC {proc}</span>
        <span className="opacity-40">&bull;</span>
        <span>
          CONN {conn}
          {connWaiting}
        </span>
        <span className="opacity-40">&bull;</span>
        <span>NET {net}</span>
      </div>

      <div className="flex items-center gap-2">
        <span>{historyCount} queries in history</span>
      </div>
    </div>
  );
}
