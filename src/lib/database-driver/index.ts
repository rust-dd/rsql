import type { MutationReport, RowMutation } from "@/lib/mutations";
import { type CellValue, decodeResult } from "@/lib/wire";
import type { SchemaIndex } from "@/monaco/completion/types";
import type {
  ColumnDetail,
  ConstraintDetail,
  DbGrant,
  FunctionInfo,
  IndexDetail,
  PgRole,
  PolicyDetail,
  ProjectConnectionStatus,
  RuleDetail,
  SchemaObject,
  TableGrant,
  TriggerDetail,
  TriggerFunctionInfo,
} from "@/types";

// Wire types from Rust (tuples)
export type WireTableInfo = [string, string];
export type WireQueryResult = [string[], CellValue[][], number];
export type WirePackedResult = [string, number]; // [packed_string, elapsed_ms]

export { CELL_SEP, type CellValue, ESC, ROW_SEP } from "@/lib/wire";

export function unpackResult(packed: string, time: number): WireQueryResult {
  const { columns, rows } = decodeResult(packed);
  return [columns, rows, time];
}
export type WireColumnDetail = [string, string, boolean, string | null];
export type WireIndexDetail = [string, string, boolean, boolean];
export type WireConstraintDetail = [string, string, string];
export type WireTriggerDetail = [string, string, string];
export type WireRuleDetail = [string, string];
export type WirePolicyDetail = [string, string, string];
export type WireFunctionInfo = [string, string, string];
export type WireTriggerFunctionInfo = [string, string];
export type WireForeignKeyInfo = [string, string, string, string, string];

export interface ForeignKey {
  sourceTable: string;
  sourceColumn: string;
  targetSchema: string;
  targetTable: string;
  targetColumn: string;
}

export type QueryStreamEvent =
  | { type: "columns"; columns: string; total_rows: number }
  | { type: "chunk"; data: string }
  | { type: "done"; elapsed: number; capped: boolean };

export interface StreamCallbacks {
  onColumns: (columns: string[], totalRows: number) => void;
  onChunk: (rows: CellValue[][]) => void;
  onDone: (elapsed: number, capped: boolean) => void;
}

export interface DatabaseDriver {
  connect(
    projectId: string,
    key: [string, string, string, string, string, string],
    ssh?: string[],
  ): Promise<ProjectConnectionStatus>;
  cancelQuery?(execId: string): Promise<boolean>;
  loadSchemas(projectId: string): Promise<string[]>;
  loadTables(projectId: string, schema: string): Promise<WireTableInfo[]>;
  loadColumns(projectId: string, schema: string, table: string): Promise<string[]>;
  loadColumnDetails(projectId: string, schema: string, table: string): Promise<ColumnDetail[]>;
  loadIndexes(projectId: string, schema: string, table: string): Promise<IndexDetail[]>;
  loadConstraints(projectId: string, schema: string, table: string): Promise<ConstraintDetail[]>;
  loadTriggers(projectId: string, schema: string, table: string): Promise<TriggerDetail[]>;
  loadRules(projectId: string, schema: string, table: string): Promise<RuleDetail[]>;
  loadPolicies(projectId: string, schema: string, table: string): Promise<PolicyDetail[]>;
  loadViews(projectId: string, schema: string): Promise<string[]>;
  loadMaterializedViews(projectId: string, schema: string): Promise<string[]>;
  loadFunctions(projectId: string, schema: string): Promise<FunctionInfo[]>;
  loadTriggerFunctions(projectId: string, schema: string): Promise<TriggerFunctionInfo[]>;
  runQuery(
    projectId: string,
    sql: string,
    timeoutMs?: number,
    execId?: string,
  ): Promise<WireQueryResult>;
  runQueryStreamed?(
    projectId: string,
    sql: string,
    streamId: string,
    callbacks: StreamCallbacks,
    execId?: string,
  ): Promise<void>;
  executeVirtual?(
    projectId: string,
    sql: string,
    queryId: string,
    pageSize: number,
    timeoutMs?: number,
    execId?: string,
  ): Promise<[string, number, string, number, boolean]>;
  fetchPage?(
    projectId: string,
    queryId: string,
    colCount: number,
    offset: number,
    limit: number,
  ): Promise<string>;
  closeVirtual?(projectId: string, queryId: string): Promise<void>;
  loadActivity(projectId: string): Promise<string[][]>;
  loadDatabaseStats(projectId: string): Promise<[string, string][]>;
  loadTableStats(projectId: string): Promise<string[][]>;
  loadForeignKeys(projectId: string, schema: string): Promise<ForeignKey[]>;
  loadTableStatistics?(
    projectId: string,
    schema: string,
    table: string,
  ): Promise<[string, string][]>;
  loadFKDetails?(
    projectId: string,
    schema: string,
    table: string,
    direction: string,
  ): Promise<[string, string, string, string, string, string, string, string, string][]>;
  loadViewInfo?(projectId: string, schema: string, view: string): Promise<[string, string][]>;
  loadMatviewInfo?(projectId: string, schema: string, matview: string): Promise<[string, string][]>;
  loadFunctionInfo?(
    projectId: string,
    schema: string,
    funcName: string,
  ): Promise<[string, string][]>;
  generateDDL?(
    projectId: string,
    schema: string,
    name: string,
    objectType: string,
  ): Promise<string>;
  csvPreview?(filePath: string): Promise<[string[], string[][]]>;
  csvImport?(
    projectId: string,
    filePath: string,
    schema: string,
    table: string,
    columnMapping: [number, string][],
  ): Promise<number>;
  listenStart?(projectId: string, channel: string): Promise<boolean>;
  listenStop?(projectId: string, channel: string): Promise<boolean>;
  notifySend?(projectId: string, channel: string, payload: string): Promise<boolean>;
  discoverChannels?(projectId: string): Promise<string[]>;
  loadRoles?(projectId: string): Promise<PgRole[]>;
  loadTableGrants?(projectId: string, roleName: string): Promise<TableGrant[]>;
  loadDatabaseGrants?(projectId: string, roleName: string): Promise<DbGrant[]>;
  extractSchemaObjects?(projectId: string, schema: string): Promise<SchemaObject[]>;
  loadLocks?(projectId: string): Promise<string[][]>;
  loadIndexUsage?(projectId: string): Promise<string[][]>;
  loadTableBloat?(projectId: string): Promise<string[][]>;
  loadDatabases?(projectId: string): Promise<string[]>;
  loadTablespaces?(projectId: string): Promise<[string, string, string][]>;
  loadExtensions?(projectId: string): Promise<string[][]>;
  loadAvailableExtensions?(projectId: string): Promise<string[][]>;
  loadEnumTypes?(projectId: string): Promise<string[][]>;
  loadPgSettings?(projectId: string): Promise<string[][]>;
  loadSchemaIndex?(projectId: string, schema: string): Promise<SchemaIndex>;
  applyRowMutations?(
    projectId: string,
    schema: string,
    table: string,
    mutations: RowMutation[],
    timeoutMs?: number,
  ): Promise<MutationReport>;
  tableAction?(
    projectId: string,
    action: string,
    schema: string,
    table: string,
    objectType: string,
  ): Promise<string>;
}

export function parseColumnDetails(wire: WireColumnDetail[]): ColumnDetail[] {
  return wire.map(([name, dataType, nullable, defaultValue]) => ({
    name,
    dataType,
    nullable,
    defaultValue,
  }));
}

export function parseIndexDetails(wire: WireIndexDetail[]): IndexDetail[] {
  return wire.map(([indexName, columnName, isUnique, isPrimary]) => ({
    indexName,
    columnName,
    isUnique,
    isPrimary,
  }));
}

export function parseConstraintDetails(wire: WireConstraintDetail[]): ConstraintDetail[] {
  return wire.map(([constraintName, constraintType, columnName]) => ({
    constraintName,
    constraintType,
    columnName,
  }));
}

export function parseTriggerDetails(wire: WireTriggerDetail[]): TriggerDetail[] {
  return wire.map(([triggerName, event, timing]) => ({
    triggerName,
    event,
    timing,
  }));
}

export function parseRuleDetails(wire: WireRuleDetail[]): RuleDetail[] {
  return wire.map(([ruleName, event]) => ({ ruleName, event }));
}

export function parsePolicyDetails(wire: WirePolicyDetail[]): PolicyDetail[] {
  return wire.map(([policyName, permissive, command]) => ({
    policyName,
    permissive,
    command,
  }));
}

export function parseFunctionInfo(wire: WireFunctionInfo[]): FunctionInfo[] {
  return wire.map(([name, returnType, arguments_]) => ({
    name,
    returnType,
    arguments: arguments_,
  }));
}

export function parseTriggerFunctionInfo(wire: WireTriggerFunctionInfo[]): TriggerFunctionInfo[] {
  return wire.map(([name, arguments_]) => ({
    name,
    arguments: arguments_,
  }));
}

export type { DriverConfig, DriverType } from "./factory";
export { DRIVER_CONFIGS, DriverFactory } from "./factory";
