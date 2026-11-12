import { RUN_LABELS, useRunScope } from "@/stores/editor-scope-store";
import type { Tab } from "@/types";

interface SplitRunBarProps {
  tab: Tab;
  onRun: () => void;
  onRunScript: () => void;
  onCancel: () => void;
}

const BUTTON = "rounded border border-border px-2 py-1 hover:bg-accent disabled:opacity-40";

export function SplitRunBar({ tab, onRun, onRunScript, onCancel }: SplitRunBarProps) {
  const run = RUN_LABELS[useRunScope(tab.id, "split")];
  const disabled = !tab.projectId || !tab.splitEditorValue?.trim();

  return (
    <div className="flex items-center justify-between border-b border-border px-3 py-1 text-xs">
      <span className="text-muted-foreground">Right query</span>
      {tab.isSplitExecuting ? (
        <button type="button" onClick={onCancel} className={BUTTON}>
          Stop right query
        </button>
      ) : (
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={onRunScript}
            disabled={disabled}
            className={BUTTON}
            title="Runs every statement in the right editor"
          >
            Run script
          </button>
          <button
            type="button"
            onClick={onRun}
            disabled={disabled}
            className={BUTTON}
            title={run.title}
          >
            {run.label}
          </button>
        </div>
      )}
    </div>
  );
}
