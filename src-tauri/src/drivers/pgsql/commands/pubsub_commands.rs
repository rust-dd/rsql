use crate::AppState;
use crate::common::enums::{AppError, query_failed};
use crate::drivers::pgsql::discover_notify_channels;
use crate::drivers::pgsql::tls::PgSslMode;

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager, Result, State};
use tokio_postgres::{AsyncMessage, Config, NoTls};

use super::pool_connection::acquire_client;

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_listen_start(project_id: &str, channel: &str, app: AppHandle) -> Result<bool> {
    let app_handle = app.clone();
    let app_state = app_handle.state::<AppState>();
    let listen_key = format!("{}:{}", project_id, channel);

    {
        let handles = app_state.notify_handles.lock().await;
        if handles.contains_key(&listen_key) {
            return Ok(true); // Already listening
        }
    }

    let (cfg, ssl) = {
        let conn = app_state
            .local_db
            .connect()
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let mut rows = conn
            .query(
                "SELECT username, password, database, host, port, ssl FROM projects WHERE id = ?1",
                libsql::params![project_id],
            )
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .ok_or_else(|| AppError::ProjectNotFound(project_id.to_string()))?;

        let mut cfg = Config::new();
        cfg.user(row.get::<String>(0).unwrap_or_default())
            .password(row.get::<String>(1).unwrap_or_default())
            .dbname(row.get::<String>(2).unwrap_or_default())
            .host(row.get::<String>(3).unwrap_or_default())
            .port(
                row.get::<String>(4)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(5432),
            );
        let ssl = PgSslMode::from_stored(&row.get::<String>(5).unwrap_or_default())?;
        ssl.apply(&mut cfg);
        (cfg, ssl.connector()?)
    };

    let channel_owned = channel.to_string();
    let event_name = format!("pg-notify-{}", project_id);

    let handle = tokio::spawn(async move {
        async fn listen_loop<S, T>(
            client: tokio_postgres::Client,
            mut connection: tokio_postgres::Connection<S, T>,
            channel: &str,
            event_name: &str,
            app: &AppHandle,
        ) where
            S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
            T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        {
            let listen_sql = format!("LISTEN \"{}\"", channel.replace('"', "\"\""));
            if let Err(e) = client.batch_execute(&listen_sql).await {
                tracing::error!("LISTEN command failed: {:?}", e);
                return;
            }
            tracing::info!("LISTEN started on channel: {}", channel);

            let mut stream = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));

            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(AsyncMessage::Notification(n)) => {
                        let payload = serde_json::json!({
                            "channel": n.channel(),
                            "payload": n.payload(),
                        });
                        let _ = app.emit(event_name, payload);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("LISTEN stream error: {:?}", e);
                        break;
                    }
                }
            }
            tracing::info!("LISTEN ended on channel: {}", channel);
            drop(client);
        }

        if let Some(tls) = ssl {
            match cfg.connect(tls).await {
                Ok((client, connection)) => {
                    listen_loop(client, connection, &channel_owned, &event_name, &app).await;
                }
                Err(e) => tracing::error!("LISTEN connect error: {:?}", e),
            }
        } else {
            match cfg.connect(NoTls).await {
                Ok((client, connection)) => {
                    listen_loop(client, connection, &channel_owned, &event_name, &app).await;
                }
                Err(e) => tracing::error!("LISTEN connect error: {:?}", e),
            }
        }
    });

    {
        let mut handles = app_state.notify_handles.lock().await;
        handles.insert(listen_key, handle);
    }

    Ok(true)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_listen_stop(project_id: &str, channel: &str, app: AppHandle) -> Result<bool> {
    let app_state = app.state::<AppState>();
    let listen_key = format!("{}:{}", project_id, channel);

    let mut handles = app_state.notify_handles.lock().await;
    if let Some(handle) = handles.remove(&listen_key) {
        handle.abort();
    }

    Ok(true)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_notify_send(
    project_id: &str,
    channel: &str,
    payload: &str,
    app_state: State<'_, AppState>,
) -> Result<bool> {
    let client = acquire_client(&app_state.clients, project_id).await?;
    let sql = format!(
        "SELECT pg_notify('{}', '{}')",
        channel.replace('\'', "''"),
        payload.replace('\'', "''"),
    );
    client.batch_execute(&sql).await.map_err(query_failed)?;
    Ok(true)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_discover_channels(
    project_id: &str,
    app_state: State<'_, AppState>,
) -> Result<Vec<String>> {
    let client = acquire_client(&app_state.meta_clients, project_id).await?;
    discover_notify_channels(&client).await.map_err(Into::into)
}
