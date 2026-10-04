import { AlignLeft, Columns2, GitBranch, Play, Save, Square, Timer } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { format as formatSQL } from "sql-formatter";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useProjectStore } from "@/stores/project-store";
import { useQueryStore } from "@/stores/query-store";
import { tabIdAt, useActiveTab, useActiveTabId, useTabStore } from "@/stores/tab-store";

const TIMEOUT_OPTIONS = [
  { label: "No limit", value: 0 },
  { label: "5s", value: 5000 },
  { label: "10s", value: 10000 },
  { label: "30s", value: 30000 },
  { label: "1m", value: 60000 },
  { label: "5m", value: 300000 },
  { label: "10m", value: 600000 },
];

export function EditorToolbar({
  onExecute,
  onExplain,
  onCancel,
}: {
  onExecute: () => void;
  onExplain: () => void;
  onCancel: () => void;
}) {
  const activeTab = useActiveTab();
  const selectedTabIndex = useTabStore((s) => s.selectedTabIndex);
  const activeTabId = useActiveTabId();
  const updateContent = useTabStore((s) => s.updateContent);
  const toggleSplit = useTabStore((s) => s.toggleSplit);
  const setQueryTimeout = useTabStore((s) => s.setQueryTimeout);
  const projects = useProjectStore((s) => s.projects);
  const saveQueryAction = useQueryStore((s) => s.saveQuery);
  const activeProject = activeTab?.projectId;
  const activeProjectDetails = activeProject ? projects[activeProject] : undefined;

  const [saveDialogOpen, setSaveDialogOpen] = useState(false);
  const [saveTitle, setSaveTitle] = useState("");
  const saveInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (saveDialogOpen) {
      setTimeout(() => saveInputRef.current?.focus(), 100);
    }
  }, [saveDialogOpen]);

  const handleSaveSubmit = async () => {
    if (!activeProject || !activeProjectDetails || !saveTitle.trim()) return;
    await saveQueryAction(
      activeProject,
      activeProjectDetails.database,
      activeProjectDetails.driver,
      saveTitle.trim(),
      activeTab?.editorValue ?? "",
    );
    setSaveTitle("");
    setSaveDialogOpen(false);
  };

  const formatQuery = () => {
    const sql = activeTab?.editorValue;
    if (!sql?.trim()) return;
    try {
      const formatted = formatSQL(sql, {
        language: "postgresql",
        tabWidth: 2,
        keywordCase: "upper",
      });
      const tabId = tabIdAt(selectedTabIndex);
      if (tabId) updateContent(tabId, formatted);
    } catch {
      // silently ignore formatting errors
    }
  };

  if (!activeTab || activeTab.type !== "query") return null;

  const hasContent = !!activeTab.editorValue?.trim();

  return (
    <>
      <div className="flex items-center gap-1 border-b border-border/40 bg-card/40 backdrop-blur-sm px-2 py-1 flex-shrink-0">
        <Button
          variant="ghost"
          size="sm"
          className="h-7 gap-1.5 text-xs px-2"
          onClick={() => setSaveDialogOpen(true)}
          disabled={!activeProject || !hasContent}
        >
          <Save className="h-3.5 w-3.5" />
          Save
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 gap-1.5 text-xs px-2"
          onClick={formatQuery}
          disabled={!hasContent}
        >
          <AlignLeft className="h-3.5 w-3.5" />
          Format
        </Button>
        <Button
          variant={activeTab?.isSplit ? "outline" : "ghost"}
          size="sm"
          className="h-7 gap-1.5 text-xs px-2"
          onClick={() => activeTabId && toggleSplit(activeTabId)}
          title="Toggle split editor"
        >
          <Columns2 className="h-3.5 w-3.5" />
          Split
        </Button>

        <div className="h-4 w-px bg-border/40 mx-1" />

        <div className="flex items-center gap-1">
          <Timer className="h-3 w-3 text-muted-foreground" />
          <select
            value={activeTab.queryTimeout ?? 0}
            onChange={(e) => activeTabId && setQueryTimeout(activeTabId, Number(e.target.value))}
            className="h-7 bg-transparent border border-border/40 rounded text-xs font-mono text-muted-foreground px-1.5 outline-none focus:border-border cursor-pointer"
            title="Query timeout"
          >
            {TIMEOUT_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        <div className="h-4 w-px bg-border/40 mx-1" />

        <Button
          variant="outline"
          size="sm"
          className="h-7 gap-1.5 text-xs px-2"
          onClick={onExplain}
          disabled={!activeProject || !hasContent || activeTab.isExecuting}
        >
          <GitBranch className="h-3.5 w-3.5" />
          Explain
          <kbd className="hidden sm:inline-flex ml-0.5 text-[10px] text-muted-foreground/60">
            {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Shift+Enter
          </kbd>
        </Button>
        {activeTab.isExecuting ? (
          <Button
            variant="outline"
            size="sm"
            className="h-7 gap-1.5 text-xs px-2.5 border-destructive/50 text-destructive hover:bg-destructive/10"
            onClick={onCancel}
          >
            <Square className="h-3 w-3" />
            Stop
            <kbd className="hidden sm:inline-flex ml-0.5 text-[10px] opacity-60">
              {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+.
            </kbd>
          </Button>
        ) : (
          <Button
            variant="gradient"
            size="sm"
            className="h-7 gap-1.5 text-xs px-2.5"
            onClick={onExecute}
            disabled={!activeProject || !hasContent}
          >
            <Play className="h-3.5 w-3.5" />
            Execute
            <kbd className="hidden sm:inline-flex ml-0.5 text-[10px] text-white/60">
              {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Enter
            </kbd>
          </Button>
        )}
      </div>

      <Dialog open={saveDialogOpen} onOpenChange={setSaveDialogOpen}>
        <DialogContent className="sm:max-w-[400px]">
          <DialogHeader>
            <DialogTitle>Save Query</DialogTitle>
            <DialogDescription>Save the current query for quick access later.</DialogDescription>
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void handleSaveSubmit();
            }}
            className="space-y-4 mt-2"
          >
            <div className="space-y-2">
              <label htmlFor="save-query-title" className="font-mono text-xs text-muted-foreground">
                Query Name
              </label>
              <Input
                id="save-query-title"
                ref={saveInputRef}
                value={saveTitle}
                onChange={(e) => setSaveTitle(e.target.value)}
                placeholder="e.g. Active users report"
                required
              />
            </div>
            <div className="rounded-lg bg-muted/50 p-2 max-h-24 overflow-auto">
              <pre className="font-mono text-[11px] text-muted-foreground whitespace-pre-wrap">
                {activeTab?.editorValue?.slice(0, 300)}
                {(activeTab?.editorValue?.length ?? 0) > 300 ? "..." : ""}
              </pre>
            </div>
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="ghost"
                onClick={() => setSaveDialogOpen(false)}
                className="text-xs"
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="gradient"
                className="text-xs"
                disabled={!saveTitle.trim()}
              >
                <Save className="h-3.5 w-3.5 mr-1" />
                Save Query
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}
