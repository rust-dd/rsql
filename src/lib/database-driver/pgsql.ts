import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MutationReport, RowMutation } from "@/lib/mutations";
import { connectWithHostKeyVerification } from "@/lib/ssh-trust";
import { decodeColumns, decodePage } from "@/lib/wire";
import type { SchemaIndex } from "@/monaco/completion/types";
import type { DbGrant, PgRole, ProjectConnectionStatus, SchemaObject, TableGrant } from "@/types";
import type {
  DatabaseDriver,
  QueryStreamEvent,
  StreamCallbacks,
  WireColumnDetail,
  WireConstraintDetail,
  WireForeignKeyInfo,
  WireFunctionInfo,
  WireIndexDetail,
  WirePackedResult,
  WirePolicyDetail,
  WireRuleDetail,
  WireTableInfo,
  WireTriggerDetail,
  WireTriggerFunctionInfo,
} from "./index";
import {
  parseColumnDetails,
  parseConstraintDetails,
  parseFunctionInfo,
  parseIndexDetails,
  parsePolicyDetails,
  parseRuleDetails,
  parseTriggerDetails,
  parseTriggerFunctionInfo,
  unpackResult,
} from "./index";

export class PostgreSQLDriver implements DatabaseDriver {
  async connect(
    projectId: string,
    key: [string, string, string, string, string, string],
    ssh?: string[],
  ) {
    return connectWithHostKeyVerification(() =>
      invoke<ProjectConnectionStatus>("pgsql_connector", {
        project_id: projectId,
        key,
        ssh: ssh ?? null,
      }),
    );
  }
  async cancelQuery(execId: string) {
    return invoke<boolean>("pgsql_cancel_query", { exec_id: execId });
  }
  async loadSchemas(projectId: string) {
    return invoke<string[]>("pgsql_load_schemas", { project_id: projectId });
  }
  async loadTables(projectId: string, schema: string) {
    return invoke<WireTableInfo[]>("pgsql_load_tables", { project_id: projectId, schema });
  }
  async loadColumns(projectId: string, schema: string, table: string) {
    return invoke<string[]>("pgsql_load_columns", { project_id: projectId, schema, table });
  }
  async loadColumnDetails(projectId: string, schema: string, table: string) {
    const wire = await invoke<WireColumnDetail[]>("pgsql_load_column_details", {
      project_id: projectId,
      schema,
      table,
    });
    return parseColumnDetails(wire);
  }
  async loadIndexes(projectId: string, schema: string, table: string) {
    const wire = await invoke<WireIndexDetail[]>("pgsql_load_indexes", {
      project_id: projectId,
      schema,
      table,
    });
    return parseIndexDetails(wire);
  }
  async loadConstraints(projectId: string, schema: string, table: string) {
    const wire = await invoke<WireConstraintDetail[]>("pgsql_load_constraints", {
      project_id: projectId,
      schema,
      table,
    });
    return parseConstraintDetails(wire);
  }
  async loadTriggers(projectId: string, schema: string, table: string) {
    const wire = await invoke<WireTriggerDetail[]>("pgsql_load_triggers", {
      project_id: projectId,
      schema,
      table,
    });
    return parseTriggerDetails(wire);
  }
  async loadRules(projectId: string, schema: string, table: string) {
    const wire = await invoke<WireRuleDetail[]>("pgsql_load_rules", {
      project_id: projectId,
      schema,
      table,
    });
    return parseRuleDetails(wire);
  }
  async loadPolicies(projectId: string, schema: string, table: string) {
    const wire = await invoke<WirePolicyDetail[]>("pgsql_load_policies", {
      project_id: projectId,
      schema,
      table,
    });
    return parsePolicyDetails(wire);
  }
  async loadViews(projectId: string, schema: string) {
    return invoke<string[]>("pgsql_load_views", { project_id: projectId, schema });
  }
  async loadMaterializedViews(projectId: string, schema: string) {
    return invoke<string[]>("pgsql_load_materialized_views", { project_id: projectId, schema });
  }
  async loadFunctions(projectId: string, schema: string) {
    const wire = await invoke<WireFunctionInfo[]>("pgsql_load_functions", {
      project_id: projectId,
      schema,
    });
    return parseFunctionInfo(wire);
  }
  async loadTriggerFunctions(projectId: string, schema: string) {
    const wire = await invoke<WireTriggerFunctionInfo[]>("pgsql_load_trigger_functions", {
      project_id: projectId,
      schema,
    });
    return parseTriggerFunctionInfo(wire);
  }
  async runQuery(projectId: string, sql: string, timeoutMs?: number, execId?: string) {
    // Use packed format for faster IPC (avoids JSON overhead of nested arrays)
    const [packed, time] = await invoke<WirePackedResult>("pgsql_run_query_packed", {
      project_id: projectId,
      sql,
      exec_id: execId ?? crypto.randomUUID(),
      timeout_ms: timeoutMs ?? null,
    });
    return unpackResult(packed, time);
  }
  async runQueryStreamed(
    projectId: string,
    sql: string,
    streamId: string,
    { onColumns, onChunk, onDone }: StreamCallbacks,
    execId?: string,
  ): Promise<void> {
    let resolveStream: () => void;
    let rejectStream: (err: unknown) => void;
    const streamDone = new Promise<void>((resolve, reject) => {
      resolveStream = resolve;
      rejectStream = reject;
    });

    const unlisten = await listen<QueryStreamEvent>(`query-stream-${streamId}`, (event) => {
      const p = event.payload;
      switch (p.type) {
        case "columns": {
          onColumns(decodeColumns(p.columns), p.total_rows);
          break;
        }
        case "chunk": {
          if (p.data) {
            onChunk(decodePage(p.data));
          }
          break;
        }
        case "done": {
          onDone(p.elapsed, p.capped);
          unlisten();
          resolveStream?.();
          break;
        }
      }
    });

    invoke("pgsql_run_query_streamed", {
      project_id: projectId,
      sql,
      stream_id: streamId,
      exec_id: execId ?? streamId,
    }).catch((err) => {
      unlisten();
      rejectStream?.(err);
    });

    return streamDone;
  }
  async executeVirtual(
    projectId: string,
    sql: string,
    queryId: string,
    pageSize: number,
    timeoutMs?: number,
    execId?: string,
  ) {
    return invoke<[string, number, string, number, boolean]>("pgsql_execute_virtual", {
      project_id: projectId,
      sql,
      query_id: queryId,
      exec_id: execId ?? queryId,
      page_size: pageSize,
      timeout_ms: timeoutMs ?? null,
    });
  }
  async fetchPage(
    _projectId: string,
    queryId: string,
    colCount: number,
    offset: number,
    limit: number,
  ) {
    return invoke<string>("pgsql_fetch_page", {
      query_id: queryId,
      col_count: colCount,
      offset,
      limit,
    });
  }
  async closeVirtual(_projectId: string, queryId: string) {
    return invoke<void>("pgsql_close_virtual", {
      query_id: queryId,
    });
  }
  async loadActivity(projectId: string) {
    return invoke<string[][]>("pgsql_load_activity", { project_id: projectId });
  }
  async loadDatabaseStats(projectId: string) {
    return invoke<[string, string][]>("pgsql_load_database_stats", { project_id: projectId });
  }
  async loadTableStats(projectId: string) {
    return invoke<string[][]>("pgsql_load_table_stats", { project_id: projectId });
  }
  async loadForeignKeys(projectId: string, schema: string) {
    const wire = await invoke<WireForeignKeyInfo[]>("pgsql_load_foreign_keys", {
      project_id: projectId,
      schema,
    });
    return wire.map(([sourceTable, sourceColumn, targetTable, targetColumn]) => ({
      sourceTable,
      sourceColumn,
      targetTable,
      targetColumn,
    }));
  }
  async loadTableStatistics(projectId: string, schema: string, table: string) {
    return invoke<[string, string][]>("pgsql_table_statistics", {
      project_id: projectId,
      schema,
      table,
    });
  }
  async loadFKDetails(projectId: string, schema: string, table: string, direction: string) {
    return invoke<[string, string, string, string, string, string, string, string, string][]>(
      "pgsql_fk_details",
      { project_id: projectId, schema, table, direction },
    );
  }
  async loadViewInfo(projectId: string, schema: string, view: string) {
    return invoke<[string, string][]>("pgsql_view_info", { project_id: projectId, schema, view });
  }
  async loadMatviewInfo(projectId: string, schema: string, matview: string) {
    return invoke<[string, string][]>("pgsql_matview_info", {
      project_id: projectId,
      schema,
      matview,
    });
  }
  async loadFunctionInfo(projectId: string, schema: string, funcName: string) {
    return invoke<[string, string][]>("pgsql_function_info", {
      project_id: projectId,
      schema,
      func_name: funcName,
    });
  }
  async generateDDL(projectId: string, schema: string, name: string, objectType: string) {
    return invoke<string>("pgsql_generate_ddl", {
      project_id: projectId,
      schema,
      name,
      object_type: objectType,
    });
  }
  async csvPreview(filePath: string) {
    return invoke<[string[], string[][]]>("pgsql_csv_preview", { file_path: filePath });
  }
  async csvImport(
    projectId: string,
    filePath: string,
    schema: string,
    table: string,
    columnMapping: [number, string][],
  ) {
    return invoke<number>("pgsql_csv_import", {
      project_id: projectId,
      file_path: filePath,
      schema,
      table,
      column_mapping: columnMapping,
    });
  }
  async listenStart(projectId: string, channel: string) {
    return invoke<boolean>("pgsql_listen_start", { project_id: projectId, channel });
  }
  async listenStop(projectId: string, channel: string) {
    return invoke<boolean>("pgsql_listen_stop", { project_id: projectId, channel });
  }
  async notifySend(projectId: string, channel: string, payload: string) {
    return invoke<boolean>("pgsql_notify_send", { project_id: projectId, channel, payload });
  }
  async discoverChannels(projectId: string) {
    return invoke<string[]>("pgsql_discover_channels", { project_id: projectId });
  }
  async loadRoles(projectId: string) {
    return invoke<PgRole[]>("pgsql_load_roles", { project_id: projectId });
  }
  async loadTableGrants(projectId: string, roleName: string) {
    return invoke<TableGrant[]>("pgsql_load_table_grants", {
      project_id: projectId,
      role_name: roleName,
    });
  }
  async loadDatabaseGrants(projectId: string, roleName: string) {
    return invoke<DbGrant[]>("pgsql_load_database_grants", {
      project_id: projectId,
      role_name: roleName,
    });
  }
  async extractSchemaObjects(projectId: string, schema: string) {
    return invoke<SchemaObject[]>("pgsql_extract_schema_objects", {
      project_id: projectId,
      schema,
    });
  }
  async loadLocks(projectId: string) {
    return invoke<string[][]>("pgsql_load_locks", { project_id: projectId });
  }
  async loadIndexUsage(projectId: string) {
    return invoke<string[][]>("pgsql_load_index_usage", { project_id: projectId });
  }
  async loadTableBloat(projectId: string) {
    return invoke<string[][]>("pgsql_load_table_bloat", { project_id: projectId });
  }
  async loadDatabases(projectId: string) {
    return invoke<string[]>("pgsql_load_databases", { project_id: projectId });
  }
  async loadTablespaces(projectId: string) {
    return invoke<[string, string, string][]>("pgsql_load_tablespaces", { project_id: projectId });
  }
  async loadExtensions(projectId: string) {
    return invoke<string[][]>("pgsql_load_extensions", { project_id: projectId });
  }
  async loadAvailableExtensions(projectId: string) {
    return invoke<string[][]>("pgsql_load_available_extensions", { project_id: projectId });
  }
  async loadSchemaIndex(projectId: string, schema: string) {
    return invoke<SchemaIndex>("pgsql_load_schema_index", {
      project_id: projectId,
      schema,
    });
  }
  async loadEnumTypes(projectId: string) {
    return invoke<string[][]>("pgsql_load_enum_types", { project_id: projectId });
  }
  async loadPgSettings(projectId: string) {
    return invoke<string[][]>("pgsql_load_pg_settings", { project_id: projectId });
  }
  async applyRowMutations(
    projectId: string,
    schema: string,
    table: string,
    mutations: RowMutation[],
    timeoutMs?: number,
  ) {
    return invoke<MutationReport>("pgsql_apply_row_mutations", {
      project_id: projectId,
      schema,
      table,
      mutations,
      timeout_ms: timeoutMs ?? null,
    });
  }
  async tableAction(
    projectId: string,
    action: string,
    schema: string,
    table: string,
    objectType: string,
  ) {
    return invoke<string>("pgsql_table_action", {
      project_id: projectId,
      action,
      schema,
      table,
      object_type: objectType,
    });
  }
}
