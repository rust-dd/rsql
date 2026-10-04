import type { EditSession } from "@/lib/mutations";
import type { CellValue } from "@/lib/wire";

export type { CellValue, EditSession };

export interface ProjectDetails {
  driver: DriverType;
  username: string;
  password: string;
  database: string;
  host: string;
  port: string;
  ssl: string;
  sshEnabled: string;
  sshHost: string;
  sshPort: string;
  sshUser: string;
  sshPassword: string;
  sshKeyPath: string;
}

export type DriverType = "PGSQL";

export type ProjectMap = Record<string, ProjectDetails>;

export type TabType =
  | "query"
  | "monitor"
  | "erd"
  | "terminal"
  | "notify"
  | "roles"
  | "schema-diff"
  | "extensions"
  | "enums"
  | "pg-settings";

export interface Tab {
  id: string;
  type: TabType;
  projectId?: string;
  schema?: string;
  title: string;
  editorValue: string;
  isExecuting: boolean;
  /** Identifies the in-flight query so cancel targets this tab's query only. */
  execId?: string;
  result?: QueryResult;
  explainResult?: ExplainPlan;
  virtualQuery?: VirtualQuery;
  queryTimeout?: number;
  isSplit?: boolean;
  splitEditorValue?: string;
  splitResult?: QueryResult;
  isSplitExecuting?: boolean;
  splitExecId?: string;
  /** Pending inline row edits. Lives on the tab so it cannot leak across tabs. */
  editSession?: EditSession;
}

export interface ExplainNode {
  "Node Type": string;
  "Relation Name"?: string;
  Alias?: string;
  "Join Type"?: string;
  "Index Name"?: string;
  "Index Cond"?: string;
  Filter?: string;
  "Hash Cond"?: string;
  "Merge Cond"?: string;
  "Sort Key"?: string[];
  Strategy?: string;
  "Startup Cost": number;
  "Total Cost": number;
  "Plan Rows": number;
  "Plan Width": number;
  "Actual Startup Time"?: number;
  "Actual Total Time"?: number;
  "Actual Rows"?: number;
  "Actual Loops"?: number;
  "Shared Hit Blocks"?: number;
  "Shared Read Blocks"?: number;
  Plans?: ExplainNode[];
  [key: string]: unknown;
}

export interface ExplainPlan {
  Plan: ExplainNode;
  "Planning Time"?: number;
  "Execution Time"?: number;
  Triggers?: unknown[];
}

export interface QueryResult {
  columns: string[];
  rows: CellValue[][];
  time: number;
  capped?: boolean;
  status?: "success" | "error" | "cancelled";
  message?: string;
}

export interface VirtualQuery {
  queryId: string;
  columns: string[];
  totalRows: number;
  pageSize: number;
  colCount: number;
  time: number;
}

export interface TableInfo {
  name: string;
  size: string;
}

export interface ColumnDetail {
  name: string;
  dataType: string;
  nullable: boolean;
  defaultValue: string | null;
}

export interface IndexDetail {
  indexName: string;
  columnName: string;
  isUnique: boolean;
  isPrimary: boolean;
}

export interface ConstraintDetail {
  constraintName: string;
  constraintType: string;
  columnName: string;
}

export interface TriggerDetail {
  triggerName: string;
  event: string;
  timing: string;
}

export interface RuleDetail {
  ruleName: string;
  event: string;
}

export interface PolicyDetail {
  policyName: string;
  permissive: string;
  command: string;
}

export interface FunctionInfo {
  name: string;
  returnType: string;
  arguments: string;
}

export interface TriggerFunctionInfo {
  name: string;
  arguments: string;
}

export interface PgRole {
  name: string;
  superuser: boolean;
  create_db: boolean;
  create_role: boolean;
  login: boolean;
  replication: boolean;
  bypass_rls: boolean;
  conn_limit: number;
  valid_until: string;
  member_of: string[];
}

export interface TableGrant {
  schema: string;
  table: string;
  grantee: string;
  privileges: string[];
}

export interface DbGrant {
  database: string;
  privilege: string;
}

export interface SchemaObject {
  object_type: string;
  name: string;
  definition: string;
}

export enum ProjectConnectionStatus {
  Connected = "Connected",
  Connecting = "Connecting",
  Disconnected = "Disconnected",
  Failed = "Failed",
}
