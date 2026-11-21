use std::time::Duration;

use super::pooled::Client;
use tokio::sync::watch;
use tokio_postgres::NoTls;

use super::ProjectTicket;
use super::executions::wait_for_cancel;
use super::pool_connection::{CONNECTION_TIMEOUT, checked_out, project_pool};
use super::session_cleanup::reset_session;
use crate::ApplicationServices;
use crate::error::{Error, query_failed};
use crate::services::tls::PgSslMode;

pub(super) fn cancelled() -> Error {
    Error::QueryFailed("Query cancelled".into())
}

impl ApplicationServices {
    pub async fn query(
        &self,
        project: &str,
        execution_id: &str,
        sql: &str,
        timeout_ms: Option<u32>,
    ) -> Result<(Vec<String>, Vec<Vec<crate::wire::Cell>>, f32), Error> {
        run_user_query(self, project, execution_id, timeout_ms, async |client| {
            crate::query::execute_query(client, sql).await
        })
        .await
    }
}

pub async fn run_user_query<T, F>(
    state: &ApplicationServices,
    project_id: &str,
    exec_id: &str,
    timeout_ms: Option<u32>,
    operation: F,
) -> Result<T, Error>
where
    F: AsyncFnOnce(&tokio_postgres::Client) -> Result<T, Error>,
{
    run_until_closed(
        state,
        project_id,
        exec_id,
        timeout_ms,
        operation,
        std::future::pending(),
    )
    .await
}

pub(crate) async fn run_until_closed<T, F>(
    state: &ApplicationServices,
    project_id: &str,
    exec_id: &str,
    timeout_ms: Option<u32>,
    operation: F,
    closed: impl Future<Output = ()>,
) -> Result<T, Error>
where
    F: AsyncFnOnce(&tokio_postgres::Client) -> Result<T, Error>,
{
    run_session(
        state, project_id, None, exec_id, timeout_ms, operation, closed,
    )
    .await
}

/// Drive this future through completion after signalling cancellation so the
/// checked-out backend is settled before returning to its owner.
pub async fn run_guarded_query<T>(
    state: &ApplicationServices,
    ticket: &ProjectTicket,
    exec_id: &str,
    timeout_ms: Option<u32>,
    operation: impl AsyncFnOnce(&tokio_postgres::Client) -> Result<T, Error>,
    cancelled: impl Future<Output = ()>,
) -> Result<T, Error> {
    run_session(
        state,
        &ticket.1,
        Some(ticket),
        exec_id,
        timeout_ms,
        operation,
        cancelled,
    )
    .await
}

async fn run_session<T>(
    state: &ApplicationServices,
    project_id: &str,
    ticket: Option<&ProjectTicket>,
    exec_id: &str,
    timeout_ms: Option<u32>,
    operation: impl AsyncFnOnce(&tokio_postgres::Client) -> Result<T, Error>,
    closed: impl Future<Output = ()>,
) -> Result<T, Error> {
    let (mut execution, pool, ssl, _route_activity) =
        register_session(state, project_id, ticket, exec_id).await?;
    let work = async {
        let client = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut execution.cancelled) => return Err(cancelled()),
            client = checked_out(&pool) => client?,
        };
        run_on_client(
            client,
            timeout_ms.unwrap_or(0),
            &mut execution.cancelled,
            ssl,
            operation,
        )
        .await
    };
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = closed => {
            state.executions.cancel(exec_id);
            let _ = work.await;
            Err(cancelled())
        }
        result = &mut work => result,
    }
}

pub(super) async fn register_session(
    state: &ApplicationServices,
    project_id: &str,
    ticket: Option<&ProjectTicket>,
    exec_id: &str,
) -> Result<
    (
        super::executions::Execution,
        std::sync::Arc<super::pooled::Pool>,
        PgSslMode,
        watch::Receiver<()>,
    ),
    Error,
> {
    let _guard = state.lifecycle.lock().await;
    if ticket.is_some_and(|t| !state.owns_ticket(t)) {
        return Err(cancelled());
    }
    let execution = state.executions.start(exec_id, project_id)?;
    if *execution.cancelled.borrow() {
        return Err(cancelled());
    }
    let pool = project_pool(&state.clients, project_id).await?;
    let ssl = state
        .routes
        .lock()
        .await
        .get(project_id)
        .map(|(_, ssl)| *ssl)
        .ok_or_else(|| Error::ClientNotConnected(project_id.to_owned()))?;
    // Keep the transport alive until server cancellation and session cleanup settle.
    let activity = state.register_activity(project_id).await;
    Ok((execution, pool, ssl, activity))
}

pub(crate) async fn run_on_client<T, F>(
    client: Client,
    timeout_ms: u32,
    cancellation: &mut watch::Receiver<bool>,
    ssl: PgSslMode,
    operation: F,
) -> Result<T, Error>
where
    F: AsyncFnOnce(&tokio_postgres::Client) -> Result<T, Error>,
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
        settle_cancelled(&client, ssl).await;
        drop(Client::take(client));
        return Err(cancelled());
    };

    let cleanup = cleanup_session(&client, ssl, cancellation).await;
    if let Err(error) = cleanup {
        drop(Client::take(client));
        tracing::warn!(%error, "Discarding query connection after session cleanup failed");
        return result.and(Err(error));
    }
    result
}

pub(super) async fn cleanup_session(
    client: &Client,
    ssl: PgSslMode,
    cancellation: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    tokio::select! {
        biased;
        _ = wait_for_cancel(cancellation) => {
            settle_cancelled(client, ssl).await;
            Err(cancelled())
        },
        reset = tokio::time::timeout(CONNECTION_TIMEOUT, reset_session(client)) => match reset {
            Ok(result) => result.map_err(query_failed),
            Err(_) => Err(Error::QueryFailed("Session reset timed out".into())),
        },
    }
}

async fn cancel_backend(client: &Client, ssl: PgSslMode) {
    let token = client.cancel_token();
    let cancel = async {
        match ssl.connector()? {
            Some(tls) => token.cancel_query(tls).await.map_err(query_failed),
            None => token.cancel_query(NoTls).await.map_err(query_failed),
        }
    };
    if let Err(error) = tokio::time::timeout(Duration::from_secs(5), cancel)
        .await
        .map_err(|error| Error::ConnectionFailed(error.to_string()))
        .and_then(|result| result)
    {
        tracing::warn!(%error, "Cancellation failed; discarding the connection");
    }
}

pub(super) async fn settle_cancelled(client: &Client, ssl: PgSslMode) {
    // PostgreSQL answers this barrier only after all earlier commands finish. A
    // CancelRequest travels over another connection and can arrive before the SQL.
    let cancel = async {
        loop {
            cancel_backend(client, ssl).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    tokio::select! { biased;
        _ = client.simple_query("") => (),
        _ = tokio::time::sleep(super::pool_connection::CONNECTION_TIMEOUT) => {
            tracing::warn!("Database cancellation did not settle before closing its connection");
        }
        _ = cancel => unreachable!(),
    }
}
