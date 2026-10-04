use std::time::Duration;

use deadpool_postgres::Client;
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tokio::sync::watch;
use tokio_postgres::NoTls;

use super::executions::wait_for_cancel;
use super::pool_connection::acquire_client;
use super::session_cleanup::reset_session;
use crate::AppState;
use crate::common::enums::{AppError, query_failed};

#[cfg(test)]
#[path = "query_session_tests.rs"]
mod tests;

fn cancelled() -> AppError {
    AppError::QueryFailed("Query cancelled".into())
}

pub(crate) async fn run_user_query<T, F>(
    state: &AppState,
    project_id: &str,
    exec_id: &str,
    timeout_ms: Option<u32>,
    operation: F,
) -> Result<T, AppError>
where
    F: AsyncFnOnce(&tokio_postgres::Client) -> Result<T, AppError>,
{
    let mut execution = state.executions.start(exec_id, project_id)?;
    let client = tokio::select! {
        biased;
        _ = wait_for_cancel(&mut execution.cancelled) => return Err(cancelled()),
        client = acquire_client(&state.clients, project_id) => client?,
    };
    let use_ssl = *state
        .client_ssl
        .lock()
        .await
        .get(project_id)
        .unwrap_or(&false);
    run_on_client(
        client,
        timeout_ms.unwrap_or(0),
        &mut execution.cancelled,
        use_ssl,
        operation,
    )
    .await
}

pub(crate) async fn run_on_client<T, F>(
    client: Client,
    timeout_ms: u32,
    cancellation: &mut watch::Receiver<bool>,
    use_ssl: bool,
    operation: F,
) -> Result<T, AppError>
where
    F: AsyncFnOnce(&tokio_postgres::Client) -> Result<T, AppError>,
{
    let result = tokio::select! {
        biased;
        _ = wait_for_cancel(cancellation) => None,
        result = async {
            client.batch_execute(&format!("SET statement_timeout = {timeout_ms}"))
                .await.map_err(query_failed)?;
            operation(&client).await
        } => Some(result),
    };

    let Some(result) = result else {
        let token = client.cancel_token();
        let cancel = async {
            if use_ssl {
                let tls = TlsConnector::builder()
                    .build()
                    .map_err(|error| AppError::ConnectionFailed(error.to_string()))?;
                token
                    .cancel_query(MakeTlsConnector::new(tls))
                    .await
                    .map_err(query_failed)
            } else {
                token.cancel_query(NoTls).await.map_err(query_failed)
            }
        };
        if let Err(error) = tokio::time::timeout(Duration::from_secs(5), cancel)
            .await
            .map_err(|error| AppError::ConnectionFailed(error.to_string()))
            .and_then(|result| result)
        {
            tracing::warn!(%error, "Cancellation failed; discarding the query connection");
        }
        drop(Client::take(client));
        return Err(cancelled());
    };

    if let Err(error) = reset_session(&client).await {
        drop(Client::take(client));
        tracing::warn!(%error, "Discarding query connection after session cleanup failed");
        return result.and(Err(query_failed(error)));
    }
    result
}
