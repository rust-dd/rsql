#[cfg(feature = "structure")]
use super::command_notices::NoticeSlot;
use super::pooled::Client;
use super::{
    ProjectTicket,
    executions::wait_for_cancel,
    pool_connection::{CONNECTION_TIMEOUT, checked_out},
    query_session::{cancelled, cleanup_session, register_session},
    tls::PgSslMode,
};
use crate::{ApplicationServices, Error, error::query_failed};
#[cfg(not(feature = "structure"))]
type NoticeSlot = ();
use tokio::sync::watch;
use tokio_postgres::Transaction;

#[derive(Debug)]
pub enum WriteOutcome<T> {
    Committed {
        value: T,
        cleanup_error: Option<Error>,
    },
    Failed(Error),
    Indeterminate(Error),
}

/// Own one transaction and await the server's COMMIT response even if its pane
/// closes. A lost response is distinguishable from both rollback and cleanup.
pub async fn run_guarded_write<T>(
    state: &ApplicationServices,
    ticket: &ProjectTicket,
    exec_id: &str,
    timeout_ms: impl Into<Option<u32>>,
    operation: impl AsyncFnOnce(&Transaction<'_>) -> Result<T, Error>,
    closed: impl Future<Output = ()>,
) -> WriteOutcome<T> {
    run_write_session(
        state,
        ticket,
        exec_id,
        timeout_ms,
        async |tx, _| operation(tx).await,
        closed,
    )
    .await
}

pub(super) async fn run_write_session<T>(
    state: &ApplicationServices,
    ticket: &ProjectTicket,
    exec_id: &str,
    timeout_ms: impl Into<Option<u32>>,
    operation: impl AsyncFnOnce(&Transaction<'_>, &NoticeSlot) -> Result<T, Error>,
    closed: impl Future<Output = ()>,
) -> WriteOutcome<T> {
    let timeout_ms = timeout_ms.into();
    let (mut execution, pool, ssl, _activity) =
        match register_session(state, &ticket.1, Some(ticket), exec_id).await {
            Ok(route) => route,
            Err(e) => return WriteOutcome::Failed(e),
        };
    let work = async {
        let client = tokio::select! { biased;
            _ = wait_for_cancel(&mut execution.cancelled) => return WriteOutcome::Failed(cancelled()),
            result = checked_out(&pool) => match result { Ok(c)=>c,Err(e)=>return WriteOutcome::Failed(e) },
        };
        #[cfg(feature = "structure")]
        let notices = client.notices.clone();
        #[cfg(not(feature = "structure"))]
        let notices = ();
        run_write(
            client,
            ssl,
            &mut execution.cancelled,
            timeout_ms,
            async |tx| operation(tx, &notices).await,
            async {},
        )
        .await
    };
    tokio::pin!(work);
    tokio::select! {biased;
        _ = closed => {state.executions.cancel(exec_id); work.await}
        result = &mut work => result,
    }
}

async fn run_write<T>(
    mut client: Client,
    ssl: PgSslMode,
    cancel: &mut watch::Receiver<bool>,
    timeout_ms: Option<u32>,
    operation: impl AsyncFnOnce(&Transaction<'_>) -> Result<T, Error>,
    after_commit: impl Future<Output = ()>,
) -> WriteOutcome<T> {
    let result = async {
        let tx=tokio::select! { biased;
            _=wait_for_cancel(cancel)=>return Err(cancelled()),
            tx=tokio_postgres::Client::transaction(&mut client)=>tx.map_err(query_failed)?,
        };
        let value = tokio::select! {biased;
            _ = wait_for_cancel(cancel) => return Err(cancelled()),
            result = async {
                if let Some(timeout_ms) = timeout_ms {
                    tx.batch_execute(&format!("SET LOCAL statement_timeout = {timeout_ms}")).await.map_err(query_failed)?;
                }
                operation(&tx).await
            } => result?,
        };
        let commit=tx.commit();
        tokio::pin!(commit);
        let response=tokio::select! {
            biased;
            result=&mut commit => result,
            _=wait_for_cancel(cancel) => match tokio::time::timeout(CONNECTION_TIMEOUT,commit).await {
                Ok(result)=>result,
                Err(_)=>return Ok(WriteOutcome::Indeterminate(Error::QueryFailed("Connection closed before COMMIT was confirmed".into()))),
            }
        };
        Ok(match response {
            Ok(())=>{
                after_commit.await;
                WriteOutcome::Committed {value,cleanup_error:None}
            }
            Err(error) if error.as_db_error().is_some_and(|db| {
                use tokio_postgres::error::{Severity, SqlState};
                !matches!(db.parsed_severity(), Some(Severity::Fatal | Severity::Panic))
                    && !db.code().code().starts_with("08")
                    && db.code()!=&SqlState::T_R_STATEMENT_COMPLETION_UNKNOWN
            }) => WriteOutcome::Failed(query_failed(error)),
            Err(error)=>WriteOutcome::Indeterminate(query_failed(error)),
        })
    }.await;
    let mut outcome = result.unwrap_or_else(WriteOutcome::Failed);
    let cleanup = cleanup_session(&client, ssl, cancel).await;
    if let Err(e) = cleanup {
        drop(Client::take(client));
        if let WriteOutcome::Committed { cleanup_error, .. } = &mut outcome {
            *cleanup_error = Some(e)
        }
    }
    outcome
}
