import { save } from "@tauri-apps/plugin-dialog";
import { open } from "@tauri-apps/plugin-fs";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { DriverFactory } from "@/lib/database-driver";
import { streamVirtualCSV } from "@/lib/stream-export";
import { useProjectStore } from "@/stores/project-store";
import { useActiveTab } from "@/stores/tab-store";

export function VirtualCsvExport({
  query,
  columns,
  capped,
}: {
  query: { queryId: string; totalRows: number; pageSize: number };
  columns: string[];
  capped: boolean;
}) {
  const tab = useActiveTab();
  const active = useRef<AbortController | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  useEffect(() => () => active.current?.abort(), []);

  const download = async () => {
    const projectId = tab?.projectId;
    const project = projectId && useProjectStore.getState().projects[projectId];
    if (!projectId || !project || active.current) return;
    const driver = DriverFactory.getDriver(project.driver);
    if (!driver.fetchPage) return;
    const fetchPage = driver.fetchPage.bind(driver);
    const controller = new AbortController();
    active.current = controller;
    setProgress(0);
    let created = false;
    try {
      const path = await save({
        title: `Export ${query.totalRows.toLocaleString()} retained rows to a new CSV file`,
        defaultPath: "results.csv",
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) return;
      controller.signal.throwIfAborted();
      const file = await open(path, { write: true, createNew: true });
      created = true;
      try {
        await streamVirtualCSV({
          ...query,
          columns,
          signal: controller.signal,
          onProgress: setProgress,
          fetchPage: (offset, limit) =>
            fetchPage(projectId, query.queryId, columns.length, offset, limit),
          write: (bytes) => file.write(bytes),
        });
      } finally {
        await file.close();
      }
      toast.success(`Exported ${query.totalRows.toLocaleString()} rows`, {
        description: capped
          ? "The query was truncated; this file contains the retained rows only."
          : undefined,
      });
    } catch (error) {
      toast.error(controller.signal.aborted ? "CSV export stopped" : "Could not export CSV", {
        description: created
          ? `The file is incomplete. ${controller.signal.aborted ? "Run export again to create a complete file." : String(error)}`
          : `${String(error)} Choose a new filename if the file already exists.`,
      });
    } finally {
      if (active.current === controller) {
        active.current = null;
        setProgress(null);
      }
    }
  };

  return progress === null ? (
    <button
      type="button"
      onClick={() => void download()}
      title={`Export all ${query.totalRows.toLocaleString()} retained rows${capped ? " (truncated result)" : ""}. Choose a new file; stopping leaves a partial CSV.`}
      className="rounded px-2 py-0.5 text-xs font-mono text-muted-foreground hover:bg-accent hover:text-foreground"
    >
      Export CSV{capped ? " (truncated)" : ""}
    </button>
  ) : (
    <div role="status" className="flex items-center gap-2 text-xs">
      <span>
        Exporting {progress.toLocaleString()} / {query.totalRows.toLocaleString()}
      </span>
      <button
        type="button"
        onClick={() => active.current?.abort()}
        className="rounded border border-border px-2 py-0.5"
      >
        Stop export
      </button>
    </div>
  );
}
