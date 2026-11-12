import { Command } from "cmdk";
import {
  AlignLeft,
  Download,
  Gauge,
  GitBranch,
  Moon,
  Pin,
  PinOff,
  Play,
  Plus,
  Save,
  Sun,
  Terminal,
  XCircle,
} from "lucide-react";
import { useTabStore } from "@/stores/tab-store";
import { useUIStore } from "@/stores/ui-store";
import type { Page } from "./types";

type UIState = ReturnType<typeof useUIStore.getState>;

export function ActionsGroup({
  onClose,
  setConnectionModalOpen,
  activeProject,
  hasQuery,
  onExecute,
  onExplain,
  onAnalyze,
  formatQuery,
  setPage,
  theme,
  toggleTheme,
  onCheckUpdates,
  pinnedResult,
  clearPinnedResult,
  activeTabResult,
}: {
  onClose: () => void;
  setConnectionModalOpen: (open: boolean) => void;
  activeProject: string | undefined;
  hasQuery: boolean;
  onExecute: () => void;
  onExplain: () => void;
  onAnalyze: () => void;
  formatQuery: () => void;
  setPage: (p: Page) => void;
  theme: UIState["theme"];
  toggleTheme: () => void;
  onCheckUpdates: () => void;
  pinnedResult: UIState["pinnedResult"];
  clearPinnedResult: () => void;
  activeTabResult: boolean;
}) {
  return (
    <Command.Group heading="Actions">
      <Command.Item
        value="New Connection"
        onSelect={() => {
          onClose();
          setConnectionModalOpen(true);
        }}
      >
        <Plus className="h-4 w-4 text-muted-foreground" />
        <span>New Connection</span>
        <span className="cmdk-meta">Connection</span>
      </Command.Item>
      <Command.Item
        value="New Query Tab"
        onSelect={() => {
          onClose();
          useTabStore.getState().openTab(activeProject);
        }}
      >
        <Plus className="h-4 w-4 text-muted-foreground" />
        <span>New Query Tab</span>
      </Command.Item>
      <Command.Item
        value="Open Terminal"
        onSelect={() => {
          onClose();
          useTabStore.getState().openTerminalTab();
        }}
      >
        <Terminal className="h-4 w-4 text-muted-foreground" />
        <span>Open Terminal</span>
        <span className="cmdk-detail">{navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+`</span>
      </Command.Item>
      {activeProject && hasQuery && (
        <>
          <Command.Item
            value="Execute Query"
            onSelect={() => {
              onClose();
              onExecute();
            }}
          >
            <Play className="h-4 w-4 text-muted-foreground" />
            <span>Execute Query</span>
            <span className="cmdk-detail">
              {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter
            </span>
          </Command.Item>
          <Command.Item
            value="Explain Query"
            onSelect={() => {
              onClose();
              onExplain();
            }}
          >
            <GitBranch className="h-4 w-4 text-muted-foreground" />
            <span>Explain Query</span>
            <span className="cmdk-detail">
              {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Shift+Enter
            </span>
          </Command.Item>
          <Command.Item
            value="Explain Analyze Query"
            onSelect={() => {
              onClose();
              onAnalyze();
            }}
          >
            <Gauge className="h-4 w-4 text-muted-foreground" />
            <span>Explain Analyze Query</span>
            <span className="cmdk-detail">executes it</span>
          </Command.Item>
          <Command.Item
            value="Cancel Query"
            onSelect={() => {
              onClose();
            }}
          >
            <XCircle className="h-4 w-4 text-muted-foreground" />
            <span>Cancel Query</span>
            <span className="cmdk-detail">
              {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+.
            </span>
          </Command.Item>
        </>
      )}
      {hasQuery && (
        <>
          <Command.Item value="Format SQL" onSelect={formatQuery}>
            <AlignLeft className="h-4 w-4 text-muted-foreground" />
            <span>Format SQL</span>
          </Command.Item>
          <Command.Item value="Save Query" onSelect={() => setPage("save-query")}>
            <Save className="h-4 w-4 text-muted-foreground" />
            <span>Save Query</span>
          </Command.Item>
        </>
      )}
      <Command.Item
        value="Toggle Theme Dark Light"
        onSelect={() => {
          onClose();
          toggleTheme();
        }}
      >
        {theme === "light" ? (
          <Moon className="h-4 w-4 text-muted-foreground" />
        ) : (
          <Sun className="h-4 w-4 text-muted-foreground" />
        )}
        <span>{theme === "light" ? "Dark Mode" : "Light Mode"}</span>
      </Command.Item>
      <Command.Item
        value="Check For Updates"
        onSelect={() => {
          onClose();
          onCheckUpdates();
        }}
      >
        <Download className="h-4 w-4 text-muted-foreground" />
        <span>Check for Updates</span>
        <span className="cmdk-meta">App</span>
      </Command.Item>
      {pinnedResult ? (
        <Command.Item
          value="Clear Pinned Result"
          onSelect={() => {
            onClose();
            clearPinnedResult();
          }}
        >
          <PinOff className="h-4 w-4 text-muted-foreground" />
          <span>Clear Pinned Result</span>
        </Command.Item>
      ) : (
        activeTabResult && (
          <Command.Item
            value="Pin Current Result"
            onSelect={() => {
              onClose();
              const tab = useTabStore.getState().tabs[useTabStore.getState().selectedTabIndex];
              if (tab?.result)
                useUIStore.getState().pinResult(tab.result, tab.editorValue.slice(0, 60));
            }}
          >
            <Pin className="h-4 w-4 text-muted-foreground" />
            <span>Pin Current Result</span>
          </Command.Item>
        )
      )}
    </Command.Group>
  );
}
