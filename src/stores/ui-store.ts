import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { CellValue } from "@/lib/wire";
import type { QueryResult } from "@/types";

interface PinnedResult {
  columns: string[];
  rows: CellValue[][];
  label: string;
}

interface UIState {
  theme: "light" | "dark";
  sidebarWidth: number;
  editorHeight: number;
  connectionModalOpen: boolean;
  pinnedResult: PinnedResult | null;

  toggleTheme: () => void;
  setTheme: (theme: "light" | "dark") => void;
  setSidebarWidth: (delta: number) => void;
  setEditorHeight: (delta: number) => void;
  setConnectionModalOpen: (open: boolean) => void;
  pinResult: (result: QueryResult, label: string) => void;
  clearPinnedResult: () => void;
}

function loadTheme(): UIState["theme"] {
  try {
    return localStorage.getItem("rsql-theme") === "dark" ? "dark" : "light";
  } catch {
    return "light";
  }
}

export const useUIStore = create<UIState>()(
  immer((set, get) => ({
    theme: loadTheme(),
    sidebarWidth: 280,
    editorHeight: 50,
    connectionModalOpen: false,
    pinnedResult: null,

    toggleTheme: () => {
      get().setTheme(get().theme === "light" ? "dark" : "light");
    },

    setTheme: (theme) => {
      if (theme === "dark") {
        document.documentElement.classList.add("dark");
      } else {
        document.documentElement.classList.remove("dark");
      }
      set({ theme });
      try {
        localStorage.setItem("rsql-theme", theme);
      } catch {
        // Theme changes remain usable when storage is blocked.
      }
    },

    setSidebarWidth: (delta) => {
      set((s) => {
        s.sidebarWidth = Math.max(180, Math.min(700, s.sidebarWidth + delta));
      });
    },

    setEditorHeight: (delta) => {
      const containerHeight = window.innerHeight - 48 - 24;
      const deltaPercent = (delta / containerHeight) * 100;
      set((s) => {
        s.editorHeight = Math.max(20, Math.min(80, s.editorHeight + deltaPercent));
      });
    },

    setConnectionModalOpen: (open) => set({ connectionModalOpen: open }),

    pinResult: (result, label) => {
      set((s) => {
        s.pinnedResult = { columns: result.columns, rows: result.rows, label };
      });
    },

    clearPinnedResult: () => set({ pinnedResult: null }),
  })),
);

if (typeof document !== "undefined") {
  document.documentElement.classList.toggle("dark", useUIStore.getState().theme === "dark");
}
