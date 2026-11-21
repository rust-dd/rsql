use super::{
    helpers::column_names,
    memory::{MemoryBudget, PackedPage},
};
use futures_util::{TryStreamExt, pin_mut};
use std::{sync::Arc, time::Instant};
use tokio_postgres::Client;

use crate::error::{Error as AppError, query_failed};

use crate::query::process_simple_messages;
use crate::wire::{pack_columns, pack_rows};

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

/// Fetch cursor batches directly or retain the last fallback rowset within the shared budget.
pub async fn stream_query(
    services: &crate::ApplicationServices,
    project: &str,
    execution: &str,
    sql: &str,
    events: tokio::sync::mpsc::Sender<QueryStreamEvent>,
) -> Result<(), AppError> {
    let budget = services.virtual_cache.lock().await.budget.clone();
    crate::services::run_until_closed(
        services,
        project,
        execution,
        None,
        async |client| execute(client, sql, &events, &budget).await,
        events.closed(),
    )
    .await
}

async fn execute(
    client: &Client,
    sql: &str,
    events: &tokio::sync::mpsc::Sender<QueryStreamEvent>,
    budget: &Arc<MemoryBudget>,
) -> Result<(), AppError> {
    let start = Instant::now();
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
                    let _ = events
                        .send(QueryStreamEvent::Columns {
                            columns: header,
                            total_rows: 0,
                        })
                        .await;
                    columns_sent = true;
                }

                if batch_rows.is_empty() {
                    break;
                }

                let packed = pack_rows(&batch_rows);
                let _ = events.send(QueryStreamEvent::Chunk { data: packed }).await;

                total_sent += batch_rows.len();
                if total_sent >= MAX_STREAM_ROWS {
                    capped = true;
                    break;
                }
            }

            if !columns_sent {
                let _ = events
                    .send(QueryStreamEvent::Columns {
                        columns: String::new(),
                        total_rows: 0,
                    })
                    .await;
            }

            client.batch_execute("CLOSE _rsql_cur").await.ok();
            client.batch_execute("COMMIT").await.ok();

            let elapsed = start.elapsed().as_millis() as f32;
            let _ = events
                .send(QueryStreamEvent::Done { elapsed, capped })
                .await;
        }
        Err(_cursor_err) => {
            client.batch_execute("ROLLBACK").await.ok();

            let capped = fallback(client, sql, events, budget).await?;
            let elapsed = start.elapsed().as_millis() as f32;
            let _ = events
                .send(QueryStreamEvent::Done { elapsed, capped })
                .await;
        }
    }

    Ok(())
}

async fn fallback(
    client: &Client,
    sql: &str,
    events: &tokio::sync::mpsc::Sender<QueryStreamEvent>,
    budget: &Arc<MemoryBudget>,
) -> Result<bool, AppError> {
    let stream = client.simple_query_raw(sql).await.map_err(query_failed)?;
    pin_mut!(stream);
    let mut pages = Vec::<PackedPage>::new();
    let mut columns = None;
    let mut count = 0usize;
    let mut affected = 0u64;
    let mut capped = false;
    while let Some(message) = stream.try_next().await.map_err(query_failed)? {
        use tokio_postgres::SimpleQueryMessage::*;
        match message {
            RowDescription(description) => {
                pages.clear();
                columns = Some(column_names(&description));
                count = 0;
                capped = false;
            }
            Row(row) if !capped && !row.is_empty() => {
                if count == MAX_STREAM_ROWS {
                    capped = true;
                    continue;
                }
                let new_page = count.is_multiple_of(CURSOR_FETCH_SIZE);
                if new_page {
                    pages.push(PackedPage::new(budget.clone()));
                }
                if pages
                    .last_mut()
                    .unwrap()
                    .push((0..row.len()).map(|i| row.get(i)), !new_page)
                {
                    count += 1;
                } else {
                    if new_page {
                        pages.pop();
                    }
                    capped = true;
                }
            }
            CommandComplete(n) if columns.is_none() => affected += n,
            _ => {}
        }
    }
    if columns.is_none() && affected > 0 {
        columns = Some(vec!["Result".into()]);
        let mut page = PackedPage::new(budget.clone());
        if page.push(
            [Some(format!("{affected} rows affected").as_str())].into_iter(),
            false,
        ) {
            pages.push(page);
            count = 1;
        } else {
            capped = true;
        }
    }
    let columns = columns.unwrap_or_default();
    let _ = events
        .send(QueryStreamEvent::Columns {
            columns: pack_columns(&columns),
            total_rows: count,
        })
        .await;
    if !columns.is_empty() && pages.is_empty() {
        let _ = events
            .send(QueryStreamEvent::Chunk {
                data: String::new(),
            })
            .await;
    }
    for mut page in pages {
        let data = std::mem::take(&mut page.data);
        if events.send(QueryStreamEvent::Chunk { data }).await.is_err() {
            break;
        }
    }
    Ok(capped)
}
