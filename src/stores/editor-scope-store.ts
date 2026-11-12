import { create } from "zustand";

export type RunScope = "statement" | "selection";
type Slot = "main" | "split";

interface EditorScopeState {
  scopes: Record<string, RunScope>;
  setScope: (tabId: string, slot: Slot, scope: RunScope | undefined) => void;
}

/** Whether Run would execute an editor's selection or the statement at its caret. */
export const useEditorScopeStore = create<EditorScopeState>()((set) => ({
  scopes: {},
  setScope: (tabId, slot, scope) =>
    set((state) => {
      const key = `${tabId}:${slot}`;
      if (state.scopes[key] === scope) return state;
      const scopes = { ...state.scopes };
      if (scope) scopes[key] = scope;
      else delete scopes[key];
      return { scopes };
    }),
}));

export function useRunScope(tabId: string | undefined, slot: Slot): RunScope {
  return useEditorScopeStore((state) => (tabId && state.scopes[`${tabId}:${slot}`]) || "statement");
}

export const RUN_LABELS: Record<RunScope, { label: string; title: string }> = {
  statement: { label: "Run statement", title: "Runs the statement at the cursor" },
  selection: { label: "Run selection", title: "Runs the selected SQL as written" },
};
