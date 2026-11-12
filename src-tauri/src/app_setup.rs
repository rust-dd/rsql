use std::{collections::BTreeMap, sync::Arc};
use tauri::Manager;
use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};
use tokio::sync::Mutex;

use crate::{
    AppState, LOCAL_DB_NAME,
    dbs::{self, migrations},
    terminal, utils,
};

pub fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(desktop)]
    if let Some(pubkey) = option_env!("TAURI_UPDATER_PUBLIC_KEY") {
        app.handle()
            .plugin(tauri_plugin_updater::Builder::new().pubkey(pubkey).build())?;
    } else {
        tracing::info!(
            "Updater disabled because TAURI_UPDATER_PUBLIC_KEY was not set at build time"
        );
    }

    let app_handle = app.handle().clone();

    tauri::async_runtime::block_on(async move {
        let db_path = if cfg!(debug_assertions) {
            LOCAL_DB_NAME.to_string()
        } else {
            let app_dir = app_handle
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");
            std::fs::create_dir_all(&app_dir).ok();
            app_dir.join(LOCAL_DB_NAME).to_string_lossy().to_string()
        };

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("Failed to open local database");

        let conn = db.connect().expect("Failed to create connection");
        dbs::schema::create_local_schema(&conn)
            .await
            .expect("Failed to create local tables");

        if let Err(error) = migrations::cleanup_legacy_snapshots(&conn).await {
            tracing::warn!(%error, "Legacy result snapshot cleanup failed");
        }

        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let backup_path = format!("{db_path}.{started}.bak");
        match dbs::project_ids::migrate_project_ids(&conn, Some(&backup_path)).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, %backup_path, "Moved projects to UUID keys"),
            Err(error) => tracing::error!(%error, "Project key migration failed"),
        }

        crate::ssh::host_keys::initialize(&conn)
            .await
            .expect("Failed to initialize SSH host keys");

        let state = AppState {
            clients: Arc::new(Mutex::new(BTreeMap::new())),
            meta_clients: Arc::new(Mutex::new(BTreeMap::new())),
            executions: Arc::new(
                crate::drivers::pgsql::commands::executions::QueryExecutions::default(),
            ),
            client_ssl: Arc::new(Mutex::new(BTreeMap::new())),
            local_db: db,
            resource_monitor: Arc::new(Mutex::new(
                utils::ResourceMonitor::new().expect("sysinfo supports every desktop target"),
            )),
            virtual_cache: Arc::new(Mutex::new(crate::drivers::pgsql::VirtualCache::new())),
            notify_handles: Arc::new(Mutex::new(BTreeMap::new())),
            ssh_tunnels: Arc::new(Mutex::new(BTreeMap::new())),
            ssh_host_keys: Arc::new(crate::ssh::host_keys::HostKeyApprovals::default()),
        };
        let cache = Arc::clone(&state.virtual_cache);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                timer.tick().await;
                crate::drivers::pgsql::result_memory::evict_idle(&cache).await;
            }
        });
        app_handle.manage(state);

        let terminal_state = terminal::TerminalState {
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        app_handle.manage(terminal_state);
    });

    let handle = app.handle();

    let app_menu = SubmenuBuilder::new(handle, "RSQL")
        .about(Some(AboutMetadata {
            name: Some("RSQL".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            copyright: Some("\u{00a9} 2025 rust-dd".into()),
            comments: Some(
                "Modern SQL client for PostgreSQL.\nBuilt with Tauri, React, and Rust.".into(),
            ),
            website: Some("https://github.com/rust-dd/rust-sql".into()),
            website_label: Some("GitHub".into()),
            ..Default::default()
        }))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let edit_menu = SubmenuBuilder::new(handle, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(handle, "View").fullscreen().build()?;

    let window_menu = SubmenuBuilder::new(handle, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let menu = MenuBuilder::new(handle)
        .items(&[&app_menu, &edit_menu, &view_menu, &window_menu])
        .build()?;

    handle.set_menu(menu)?;

    #[cfg(debug_assertions)]
    {
        let window = app
            .get_webview_window("main")
            .expect("main window not found");
        window.open_devtools();
        window.close_devtools();
    }

    Ok(())
}
