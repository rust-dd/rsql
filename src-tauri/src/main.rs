// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_setup;
mod common;
mod dbs;
mod drivers;
mod ssh;
mod terminal;
mod utils;

const LOCAL_DB_NAME: &str = "rsql.db";

use deadpool_postgres::Pool;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;
use tracing::Level;

pub struct AppState {
    pub clients: Arc<Mutex<BTreeMap<String, Arc<Pool>>>>,
    pub meta_clients: Arc<Mutex<BTreeMap<String, Arc<Pool>>>>,
    /// Keyed by exec id, not project: a project can have several queries in
    /// flight and cancelling must hit the one the user asked for.
    pub(crate) executions: Arc<drivers::pgsql::commands::executions::QueryExecutions>,
    pub client_ssl: Arc<Mutex<BTreeMap<String, bool>>>,
    pub local_db: libsql::Database,
    pub resource_monitor: Arc<Mutex<utils::ResourceMonitor>>,
    pub virtual_cache: Arc<Mutex<drivers::pgsql::VirtualCache>>,
    pub notify_handles: Arc<Mutex<BTreeMap<String, tokio::task::JoinHandle<()>>>>,
    pub ssh_tunnels: Arc<Mutex<BTreeMap<String, ssh::SshTunnel>>>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .with_max_level(Level::INFO)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .setup(app_setup::setup_app)
        .invoke_handler(tauri::generate_handler![
            dbs::project::project_db_select,
            dbs::project::project_db_insert,
            dbs::project::project_db_delete,
            dbs::query::query_db_select,
            dbs::query::query_db_insert,
            dbs::query::query_db_delete,
            dbs::workspace::workspace_save,
            dbs::workspace::workspace_load_all,
            dbs::workspace::workspace_delete,
            drivers::pgsql::pgsql_test_connection,
            drivers::pgsql::pgsql_connector,
            drivers::pgsql::pgsql_load_databases,
            drivers::pgsql::pgsql_load_tablespaces,
            drivers::pgsql::pgsql_load_schemas,
            drivers::pgsql::pgsql_load_tables,
            drivers::pgsql::pgsql_load_columns,
            drivers::pgsql::pgsql_load_column_details,
            drivers::pgsql::pgsql_load_indexes,
            drivers::pgsql::pgsql_load_constraints,
            drivers::pgsql::pgsql_load_triggers,
            drivers::pgsql::pgsql_load_rules,
            drivers::pgsql::pgsql_load_policies,
            drivers::pgsql::pgsql_load_views,
            drivers::pgsql::pgsql_load_materialized_views,
            drivers::pgsql::pgsql_load_functions,
            drivers::pgsql::pgsql_load_trigger_functions,
            drivers::pgsql::pgsql_run_query,
            drivers::pgsql::pgsql_cancel_query,
            drivers::pgsql::pgsql_load_activity,
            drivers::pgsql::pgsql_load_database_stats,
            drivers::pgsql::pgsql_load_table_stats,
            drivers::pgsql::pgsql_load_foreign_keys,
            drivers::pgsql::pgsql_run_query_packed,
            drivers::pgsql::pgsql_run_query_streamed,
            drivers::pgsql::pgsql_execute_virtual,
            drivers::pgsql::pgsql_fetch_page,
            drivers::pgsql::pgsql_close_virtual,
            drivers::pgsql::pgsql_table_statistics,
            drivers::pgsql::pgsql_fk_details,
            drivers::pgsql::pgsql_view_info,
            drivers::pgsql::pgsql_matview_info,
            drivers::pgsql::pgsql_function_info,
            drivers::pgsql::pgsql_generate_ddl,
            drivers::pgsql::pgsql_csv_preview,
            drivers::pgsql::pgsql_csv_import,
            drivers::pgsql::pgsql_listen_start,
            drivers::pgsql::pgsql_listen_stop,
            drivers::pgsql::pgsql_notify_send,
            drivers::pgsql::pgsql_discover_channels,
            drivers::pgsql::pgsql_load_roles,
            drivers::pgsql::pgsql_load_table_grants,
            drivers::pgsql::pgsql_load_database_grants,
            drivers::pgsql::pgsql_extract_schema_objects,
            drivers::pgsql::pgsql_load_locks,
            drivers::pgsql::pgsql_load_index_usage,
            drivers::pgsql::pgsql_load_table_bloat,
            drivers::pgsql::pgsql_load_extensions,
            drivers::pgsql::pgsql_load_available_extensions,
            drivers::pgsql::pgsql_load_enum_types,
            drivers::pgsql::pgsql_table_action,
            drivers::pgsql::pgsql_apply_row_mutations,
            drivers::pgsql::pgsql_load_pg_settings,
            drivers::pgsql::pgsql_load_schema_index,
            terminal::terminal_spawn,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_kill,
            utils::compute_diff,
            utils::system_resource_usage,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
