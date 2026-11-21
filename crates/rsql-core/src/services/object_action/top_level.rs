use super::*;
use crate::services::pooled::Client as Checkout;
use crate::services::{
    executions::wait_for_cancel,
    pool_connection::{CONNECTION_TIMEOUT, checked_out},
    query_session::{cancelled, cleanup_session, register_session, settle_cancelled},
    tls::PgSslMode,
};
use tokio::sync::watch;

pub(super) async fn run(
    state: &ApplicationServices,
    ticket: &ProjectTicket,
    id: &str,
    review: ActionTarget,
    closed: impl Future<Output = ()>,
) -> ActionOutcome {
    let (mut execution, pool, ssl, _activity) =
        match register_session(state, &ticket.1, Some(ticket), id).await {
            Ok(v) => v,
            Err(e) => return WriteOutcome::Failed(e).into(),
        };
    let work = async {
        let client = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut execution.cancelled) => return WriteOutcome::Failed(cancelled()).into(),
            acquired = checked_out(&pool) => match acquired {
                Ok(client) => client,
                Err(error) => return WriteOutcome::Failed(error).into(),
            }
        };
        on_checkout(client, ssl, &mut execution.cancelled, &review, async |_| {}).await
    };
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = closed => {
            state.executions.cancel(id);
            work.await
        },
        outcome = &mut work => outcome,
    }
}
async fn on_checkout(
    client: Checkout,
    ssl: PgSslMode,
    cancel: &mut watch::Receiver<bool>,
    review: &ActionTarget,
    after: impl AsyncFnOnce(&Client),
) -> ActionOutcome {
    let operation = async {
        let pre = async {
            client
                .batch_execute("SET search_path = pg_catalog; SET client_min_messages = warning")
                .await
                .map_err(query_failed)?;
            if review.action != ObjectAction::Reindex {
                review.check_maintenance(&client).await?;
            }
            review.verify(&client).await?;
            client.notices.begin()
        };
        let watch = tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => return WriteOutcome::Failed(cancelled()).into(),
            result = pre => match result {
                Ok(scope) => scope,
                Err(error) => return WriteOutcome::Failed(error).into(),
            },
        };
        let command = client.batch_execute(&review.sql);
        tokio::pin!(command);
        let response = tokio::select! {
            biased;
            result = &mut command => result,
            _ = wait_for_cancel(cancel) => {
                settle_cancelled(&client, ssl).await;
                match tokio::time::timeout(CONNECTION_TIMEOUT, command).await {
                    Ok(result) => result,
                    Err(_) => return ActionOutcome {
                        result: WriteOutcome::Indeterminate(Error::QueryFailed(
                            "The server did not confirm completion.".into(),
                        )),
                        possible_effects: true,
                        server_warnings: watch.finish(),
                        verification_warning: None,
                    },
                }
            }
        };
        let server_warning = watch.finish();
        if let Err(error) = response {
            use tokio_postgres::error::{Severity, SqlState};
            let known_failure = error.as_db_error().is_some_and(|db| {
                !matches!(
                    db.parsed_severity(),
                    Some(Severity::Fatal | Severity::Panic)
                ) && !db.code().code().starts_with("08")
                    && db.code() != &SqlState::T_R_STATEMENT_COMPLETION_UNKNOWN
                    && db.code() != &SqlState::QUERY_CANCELED
            });
            let message = query_failed(error);
            return ActionOutcome {
                possible_effects: true,
                server_warnings: server_warning,
                verification_warning: None,
                result: if known_failure {
                    WriteOutcome::Failed(message)
                } else {
                    WriteOutcome::Indeterminate(message)
                },
            };
        }
        after(&client).await;
        let warning = tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => Some("Target verification was cancelled.".into()),
            result = review.verify(&client) => result.err().map(|error| format!("Target verification: {error}")),
        };
        ActionOutcome {
            result: WriteOutcome::Committed {
                value: (),
                cleanup_error: None,
            },
            server_warnings: server_warning,
            verification_warning: warning,
            possible_effects: false,
        }
    };
    let mut outcome = operation.await;
    let reset = cleanup_session(&client, ssl, cancel).await;
    if let Err(e) = reset {
        drop(Checkout::take(client));
        if let WriteOutcome::Committed { cleanup_error, .. } = &mut outcome.result {
            *cleanup_error = Some(e);
        }
    }
    outcome
}
