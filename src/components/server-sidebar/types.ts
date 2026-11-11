import type React from "react";
import type { ContextMenuEntry } from "@/components/ui/context-menu";
import type {
  ColumnDetail,
  ConstraintDetail,
  FunctionInfo,
  IndexDetail,
  PolicyDetail,
  ProjectConnectionStatus,
  ProjectMap,
  RuleDetail,
  TableInfo,
  TriggerDetail,
  TriggerFunctionInfo,
} from "@/types";

export type ObjectKind = "table" | "view" | "matview" | "function" | "trigger-function";

export type CsvImportTarget = {
  projectId: string;
  schema: string;
  table: string;
  columns: string[];
};

export type PropsModalState = {
  open: boolean;
  objectType: ObjectKind;
  projectId: string;
  schema: string;
  name: string;
};

/**
 * Bundle of state slices + handlers passed to render helpers.
 * This is plain prop-drilling, NOT React Context.
 */
export interface SidebarRenderCtx {
  projects: ProjectMap;
  status: Record<string, ProjectConnectionStatus>;
  serverDatabases: Record<string, string[]>;
  serverTablespaces: Record<string, [string, string, string][]>;
  schemas: Record<string, string[]>;
  tables: Record<string, TableInfo[]>;
  columnDetails: Record<string, ColumnDetail[]>;
  indexes: Record<string, IndexDetail[]>;
  constraints: Record<string, ConstraintDetail[]>;
  triggers: Record<string, TriggerDetail[]>;
  rules: Record<string, RuleDetail[]>;
  policies: Record<string, PolicyDetail[]>;
  views: Record<string, string[]>;
  materializedViews: Record<string, string[]>;
  functions: Record<string, FunctionInfo[]>;
  triggerFunctions: Record<string, TriggerFunctionInfo[]>;

  connect: (projectId: string) => Promise<void>;
  loadColumns: (projectId: string, schema: string, table: string) => Promise<string[]>;
  refreshConnection: (projectId: string) => Promise<void>;
  deleteProject: (projectId: string) => Promise<void>;
  addDatabaseToServer: (sourceProjectId: string, name: string, database: string) => Promise<void>;
  openTab: (projectId?: string, sql?: string) => void;
  openMonitorTab: (projectId: string) => void;
  openERDTab: (projectId: string, schema: string) => void;
  openNotifyTab: (projectId: string) => void;
  openRolesTab: (projectId: string) => void;
  openSchemaDiffTab: (projectId: string) => void;
  openExtensionsTab: (projectId: string) => void;
  openEnumsTab: (projectId: string) => void;
  openPgSettingsTab: (projectId: string) => void;

  loading: Record<string, boolean>;
  selectedItem: string | null;
  setSelectedItem: (k: string | null) => void;
  setCsvImportTarget: (t: CsvImportTarget | null) => void;
  setAddDbSource: (id: string | null) => void;
  setRenameServerPids: (pids: string[] | null) => void;
  openProperties: (objectType: ObjectKind, projectId: string, schema: string, name: string) => void;

  toggle: (key: string) => void;
  isOpen: (key: string, defaultOpen?: boolean) => boolean;
  onConnect: (projectId: string) => Promise<void>;
  onExpandSchema: (projectId: string, schema: string) => Promise<void>;
  onExpandTable: (projectId: string, schema: string, table: string) => Promise<void>;
  onOpenTableQuery: (projectId: string, schema: string, table: string) => void;

  copy: (text: string) => void;
  showMenu: (e: React.MouseEvent, items: ContextMenuEntry[]) => void;
  onEditConnection?: (projectId: string) => void;
}
