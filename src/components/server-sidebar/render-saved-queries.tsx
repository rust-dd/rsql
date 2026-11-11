import { Copy, FileText, Trash2 } from "lucide-react";
import { projectName } from "@/stores/project-store";
import type { SavedQuery } from "@/stores/query-store";
import { I } from "./constants";
import { TreeRow } from "./tree-row";
import type { SidebarRenderCtx } from "./types";

export function renderSavedQueries(
  ctx: SidebarRenderCtx,
  savedQueries: SavedQuery[],
  removeQuery: (id: string) => Promise<void>,
) {
  const { projects, selectedItem, setSelectedItem, openTab, showMenu, copy } = ctx;
  return (
    <div className="border-t border-sidebar-border">
      <div className="flex h-8 items-center justify-between px-3">
        <span className="tracking-widest uppercase text-[10px] font-semibold text-sidebar-foreground">
          SAVED QUERIES
        </span>
        {savedQueries.length > 0 && (
          <span className="text-[10px] text-muted-foreground">{savedQueries.length}</span>
        )}
      </div>
      {savedQueries.length > 0 ? (
        <div className="overflow-y-auto p-1 max-h-48">
          {savedQueries.map((q) => (
            <TreeRow
              key={q.id}
              indent={I.server}
              icon={<FileText className="h-3.5 w-3.5 text-primary/60" />}
              label={q.title}
              selected={selectedItem === `query::${q.id}`}
              onClick={() => {
                setSelectedItem(`query::${q.id}`);
                openTab(q.projectId, q.sql);
              }}
              onContextMenu={(e) => {
                setSelectedItem(`query::${q.id}`);
                showMenu(e, [
                  {
                    label: "Open in Tab",
                    icon: <FileText className="h-3 w-3" />,
                    onClick: () => openTab(q.projectId, q.sql),
                  },
                  {
                    label: "Copy SQL",
                    icon: <Copy className="h-3 w-3" />,
                    onClick: () => copy(q.sql),
                  },
                  { separator: true as const },
                  {
                    label: "Delete",
                    icon: <Trash2 className="h-3 w-3" />,
                    onClick: () => void removeQuery(q.id),
                    destructive: true,
                  },
                ]);
              }}
              trailing={
                <span className="font-mono text-[10px] text-muted-foreground shrink-0">
                  {projectName(projects, q.projectId)}
                </span>
              }
            />
          ))}
        </div>
      ) : (
        <div className="px-3 pb-2 text-[11px] text-muted-foreground/60">
          No saved queries yet. Use the Save button in the toolbar to save the current query.
        </div>
      )}
    </div>
  );
}
