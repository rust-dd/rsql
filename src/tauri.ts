import { invoke } from "@tauri-apps/api/core";

// Raw wire types from Rust backend (string arrays)
export type RawProjectMap = Record<string, string[]>;

export interface SystemResourceUsage {
  app_cpu_percent: number;
  app_memory_rss_mb: number;
  app_process_count: number;
  network_rx_mbps: number;
  network_tx_mbps: number;
  db_connections_in_use: number;
  db_connections_open: number;
  db_connections_max: number;
  db_connections_waiting: number;
}

export async function getProjects(): Promise<RawProjectMap> {
  return await invoke<RawProjectMap>("project_db_select");
}

export async function insertProject(project_id: string, project_details: string[]): Promise<void> {
  await invoke("project_db_insert", { project_id, project_details });
}

export async function setServerAlias(project_ids: string[], alias: string): Promise<void> {
  await invoke("project_db_set_server_alias", { project_ids, alias });
}

export async function deleteProject(project_id: string): Promise<void> {
  await invoke("project_db_delete", { project_id });
}

export async function getQueries(): Promise<Record<string, string>> {
  return await invoke<Record<string, string>>("query_db_select");
}

export async function insertQuery(query_id: string, sql: string): Promise<void> {
  await invoke("query_db_insert", { query_id, sql });
}

export async function deleteQuery(query_id: string): Promise<void> {
  await invoke("query_db_delete", { query_id });
}

export async function getSystemResourceUsage(): Promise<SystemResourceUsage> {
  return await invoke<SystemResourceUsage>("system_resource_usage");
}

export async function workspaceSave(name: string, tabs: string): Promise<void> {
  await invoke("workspace_save", { name, tabs });
}

export async function workspaceLoadAll(): Promise<[string, string][]> {
  return await invoke<[string, string][]>("workspace_load_all");
}

export async function workspaceDelete(name: string): Promise<void> {
  await invoke("workspace_delete", { name });
}

export async function pgsqlTestConnection(
  key: [string, string, string, string, string, string],
  ssh?: string[],
): Promise<string> {
  return await invoke<string>("pgsql_test_connection", { key, ssh: ssh ?? null });
}

export interface HistoryRecord {
  projectId: string;
  database: string;
  sql: string;
  executedAt: number;
  durationMs: number;
  rowCount: number;
  success: boolean;
  error?: string;
}

export interface StoredHistoryEntry extends Omit<HistoryRecord, "error"> {
  id: number;
  sqlTruncated: boolean;
  error: string | null;
}

export interface HistoryFilter {
  text?: string;
  projectId?: string;
  database?: string;
  success?: boolean;
  since?: number;
  beforeId?: number;
  limit?: number;
}

export interface HistoryPage {
  entries: StoredHistoryEntry[];
  hasMore: boolean;
}

export interface HistoryStatus {
  enabled: boolean;
  entries: number;
  bytes: number;
  maxEntries: number;
  maxAgeDays: number;
  maxBytes: number;
}

/** Resolves to false when history recording is turned off. */
export async function historyRecord(entry: HistoryRecord): Promise<boolean> {
  return await invoke<boolean>("history_record", { entry });
}

export async function historySearch(filter: HistoryFilter): Promise<HistoryPage> {
  return await invoke<HistoryPage>("history_search", { filter });
}

export async function historyDatabases(projectId?: string): Promise<string[]> {
  return await invoke<string[]>("history_databases", { project_id: projectId ?? null });
}

export async function historyStatus(): Promise<HistoryStatus> {
  return await invoke<HistoryStatus>("history_status");
}

export async function historySetEnabled(enabled: boolean): Promise<void> {
  await invoke("history_set_enabled", { enabled });
}

export async function historyClear(): Promise<void> {
  await invoke("history_clear");
}
