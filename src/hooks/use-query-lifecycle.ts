import { useEffect } from "react";
import { cancelTabQuery, executeTabQuery } from "@/lib/query-execution";
import { useTabStore } from "@/stores/tab-store";

interface UseQueryLifecycleArgs {
  setCommandPaletteOpen: (updater: (v: boolean) => boolean) => void;
}

const runQuery = () => executeTabQuery();
const runExplain = () => executeTabQuery("explain");
const runSplitQuery = () => executeTabQuery("split");
const cancelQuery = () => cancelTabQuery();
const cancelSplitQuery = () => cancelTabQuery(true);

export function useQueryLifecycle({ setCommandPaletteOpen }: UseQueryLifecycleArgs) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "w") {
        e.preventDefault();
        const store = useTabStore.getState();
        store.closeTab(store.selectedTabIndex);
      }
      if (e.shiftKey && e.key === "Enter") {
        e.preventDefault();
        void runExplain();
      }
      if (e.key === "p" || e.key === "k") {
        e.preventDefault();
        setCommandPaletteOpen((v) => !v);
      }
      if (e.key === "`") {
        e.preventDefault();
        useTabStore.getState().openTerminalTab();
      }
      if (e.key === ".") {
        e.preventDefault();
        const store = useTabStore.getState();
        if (store.tabs[store.selectedTabIndex]?.isExecuting) cancelQuery();
        else cancelSplitQuery();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [setCommandPaletteOpen]);

  return { runQuery, runExplain, cancelQuery, runSplitQuery, cancelSplitQuery };
}
