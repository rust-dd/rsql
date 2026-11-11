import { Database, Download, Moon, Search, Sun } from "lucide-react";
import { Button } from "@/components/ui/button";
import { projectName, useProjectStore } from "@/stores/project-store";
import { tabIdAt, useActiveTab, useTabStore } from "@/stores/tab-store";
import { useUIStore } from "@/stores/ui-store";
import { ProjectConnectionStatus } from "@/types";

export function TopBar({
  onCheckUpdates,
  onOpenCommandPalette,
}: {
  onCheckUpdates: () => void;
  onOpenCommandPalette: () => void;
}) {
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const projects = useProjectStore((s) => s.projects);
  const status = useProjectStore((s) => s.status);
  const selectedTabIndex = useTabStore((s) => s.selectedTabIndex);
  const activeTab = useActiveTab();
  const setProjectId = useTabStore((s) => s.setProjectId);
  const activeProject = activeTab?.projectId;
  const activeProjectDetails = activeProject ? projects[activeProject] : undefined;

  return (
    <div className="flex h-11 items-center justify-between border-b border-border/50 bg-card/80 backdrop-blur-xl px-4">
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2">
          <Database className="h-4 w-4 text-primary" />
          <span className="font-mono text-sm font-semibold">RSQL</span>
        </div>
        <div className="h-4 w-px bg-border/50" />
        {activeProject &&
        activeProjectDetails &&
        status[activeProject] === ProjectConnectionStatus.Connected ? (
          <div className="flex items-center gap-1.5 bg-accent rounded-full px-2.5 py-0.5 text-xs text-muted-foreground">
            <div className="h-2 w-2 rounded-full bg-success shadow-[0_0_6px_oklch(0.65_0.18_150)]" />
            <span className="font-mono">{projectName(projects, activeProject)}</span>
            <span className="text-muted-foreground/50">&bull;</span>
            <span>
              {activeProjectDetails.host}:{activeProjectDetails.port}
            </span>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            {Object.keys(projects).length > 0 ? (
              <select
                className="bg-input border border-border/50 text-foreground font-mono text-xs rounded-lg px-2 py-1"
                value={activeProject ?? ""}
                onChange={(e) => {
                  if (e.target.value) {
                    const tabId = tabIdAt(selectedTabIndex);
                    if (tabId) setProjectId(tabId, e.target.value);
                  }
                }}
              >
                <option value="">Select connection...</option>
                {Object.entries(projects).map(([id, details]) => (
                  <option key={id} value={id}>
                    {projectName(projects, id)} ({details.database})
                    {status[id] === ProjectConnectionStatus.Connected ? "" : " \u2022 disconnected"}
                  </option>
                ))}
              </select>
            ) : (
              <span className="text-xs text-muted-foreground">No connection</span>
            )}
          </div>
        )}
      </div>

      <div className="flex items-center gap-2">
        {/* Command palette trigger */}
        <button
          type="button"
          onClick={onOpenCommandPalette}
          className="flex items-center gap-2 h-7 px-3 rounded-lg border border-border/50 bg-muted/50 text-muted-foreground hover:text-foreground hover:bg-accent/50 transition-colors"
        >
          <Search className="h-3 w-3" />
          <span className="text-xs font-mono">Search...</span>
          <kbd className="hidden sm:inline-flex h-5 items-center gap-0.5 rounded border border-border/60 bg-muted px-1.5 font-mono text-[10px] text-muted-foreground/70">
            {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}K
          </kbd>
        </button>

        <div className="h-4 w-px bg-border/50" />

        <Button variant="ghost" size="sm" className="h-8 gap-2" onClick={onCheckUpdates}>
          <Download className="h-4 w-4" />
          <span className="text-xs">Updates</span>
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 hover:rotate-12 transition-all duration-200"
          onClick={toggleTheme}
        >
          {theme === "light" ? <Moon className="h-4 w-4" /> : <Sun className="h-4 w-4" />}
        </Button>
      </div>
    </div>
  );
}
