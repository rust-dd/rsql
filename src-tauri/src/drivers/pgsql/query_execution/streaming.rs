use std::time::Instant;
use tokio_postgres::Client;

use crate::common::enums::{AppError, query_failed};

use super::super::wire::{pack_columns, pack_rows};
use super::helpers::process_simple_messages;

/// Events emitted during streamed query execution.
#[derive(serde::Serialize, Clone)]
#[serde(tag = "type")]
pub enum QueryStreamEvent {
    #[serde(rename = "columns")]
    Columns { columns: String, total_rows: usize },
    #[serde(rename = "chunk")]
    Chunk { data: String },
    #[serde(rename = "done")]
    Done { elapsed: f32, capped: bool },
}

/// Maximum rows to send to the frontend to prevent OOM in the webview.
const MAX_STREAM_ROWS: usize = 500_000;
const CURSOR_FETCH_SIZE: usize = 10_000;

/// Stream query results using a PostgreSQL cursor.
/// Fetches rows in batches from the server — never loads the full result into Rust memory.
/// Caps at MAX_STREAM_ROWS to protect the webview from OOM.
pub async fn execute_query_streamed(
    client: &Client,
    sql: &str,
    stream_id: &str,
    app: &tauri::AppHandle,
) -> Result<(), AppError> {
    use tauri::Emitter;

    let start = Instant::now();
    let event_name = format!("query-stream-{}", stream_id);

    client.batch_execute("BEGIN").await.map_err(query_failed)?;

    let cursor_sql = format!("DECLARE _rsql_cur NO SCROLL CURSOR FOR {}", sql);
    match client.batch_execute(&cursor_sql).await {
        Ok(_) => {
            let fetch_sql = format!("FETCH {} FROM _rsql_cur", CURSOR_FETCH_SIZE);
            let mut total_sent = 0usize;
            let mut columns_sent = false;
            let mut capped = false;

            loop {
                let messages = match client.simple_query(&fetch_sql).await {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        let _ = client.batch_execute("CLOSE _rsql_cur; ROLLBACK").await;
                        return Err(AppError::QueryFailed(e.to_string()));
                    }
                };

                let (batch_columns, batch_rows) = process_simple_messages(messages);

                if !columns_sent {
                    let header = pack_columns(&batch_columns);
                    let _ = app.emit(
                        &event_name,
                        QueryStreamEvent::Columns {
                            columns: header,
                            total_rows: 0,
                        },
                    );
                    columns_sent = true;
                }

                if batch_rows.is_empty() {
                    break;
                }

                let packed = pack_rows(&batch_rows);
                let _ = app.emit(&event_name, QueryStreamEvent::Chunk { data: packed });

                total_sent += batch_rows.len();
                if total_sent >= MAX_STREAM_ROWS {
                    capped = true;
                    break;
                }
            }

            if !columns_sent {
                let _ = app.emit(
                    &event_name,
                    QueryStreamEvent::Columns {
                        columns: String::new(),
                        total_rows: 0,
                    },
                );
            }

            client.batch_execute("CLOSE _rsql_cur").await.ok();
            client.batch_execute("COMMIT").await.ok();

            let elapsed = start.elapsed().as_millis() as f32;
            let _ = app.emit(&event_name, QueryStreamEvent::Done { elapsed, capped });
        }
        Err(_cursor_err) => {
            client.batch_execute("ROLLBACK").await.ok();

            let messages = client.simple_query(sql).await.map_err(query_failed)?;

            let (columns, rows) = process_simple_messages(messages);

            if columns.is_empty() {
                let _ = app.emit(
                    &event_name,
                    QueryStreamEvent::Columns {
                        columns: String::new(),
                        total_rows: 0,
                    },
                );
            } else {
                let header = pack_columns(&columns);
                let _ = app.emit(
                    &event_name,
                    QueryStreamEvent::Columns {
                        columns: header,
                        total_rows: rows.len(),
                    },
                );

                let packed = pack_rows(&rows);
                let _ = app.emit(&event_name, QueryStreamEvent::Chunk { data: packed });
            }

            let elapsed = start.elapsed().as_millis() as f32;
            let _ = app.emit(
                &event_name,
                QueryStreamEvent::Done {
                    elapsed,
                    capped: false,
                },
            );
        }
    }

    Ok(())
}
