import type { CellValue } from "@/lib/wire";
import type { QueryResult } from "@/types";

export type PanelView = "grid" | "record" | "history" | "explain" | "diff" | "map";

export interface ToolbarProps {
  panelView: PanelView;
  setPanelView: (v: PanelView) => void;
  result: QueryResult | null;
  columns: string[];
  filteredRows: CellValue[][];
  searchTerm: string;
  setSearchTerm: (v: string) => void;
  filteredCount: number;
  hasExplain: boolean;
  isExecuting: boolean;
  isEditing: boolean;
  editableTable: boolean;
  isCommitting: boolean;
  editError: string | null;
  pending: { updates: number; deletes: number };
  sessionMatchesEditor: boolean;
  confirmingApply: boolean;
  onEnterEdit: () => void;
  onRequestApply: () => void;
  onConfirmApply: () => void;
  onCancelApply: () => void;
  onDiscard: () => void;
  onCancel?: () => void;
  virtualQuery?: { queryId: string; totalRows: number; time: number; pageSize: number };
}
