use crate::{
    ApplicationServices, Error,
    error::query_failed,
    services::{ProjectTicket, acquire_client, run_guarded_query},
};
use tokio_postgres::{
    error::{Severity, SqlState},
    types::Type,
};

const NOTIFY: &str = "SELECT pg_notify($1, $2)";

impl ApplicationServices {
    pub async fn notify(&self, project: &str, channel: &str, payload: &str) -> Result<(), Error> {
        let client = acquire_client(&self.clients, project).await?;
        client
            .execute(NOTIFY, &[&channel, &payload])
            .await
            .map(|_| ())
            .map_err(query_failed)
    }

    /// Send once on the captured route and settle the checkout before returning.
    pub async fn notify_guarded(
        &self,
        ticket: &ProjectTicket,
        channel: &str,
        payload: &str,
        cancelled: impl Future<Output = ()>,
    ) -> Result<(), Error> {
        let execution = uuid::Uuid::new_v4().to_string();
        let mut executing = false;
        let mut reply = None;
        let settled = run_guarded_query(
            self,
            ticket,
            &execution,
            None,
            async |client| {
                // PostgreSQL cannot accept a Bind message this large. Reject it before encoding.
                if channel.len().saturating_add(payload.len()) >= 1 << 30 {
                    return Err(Error::QueryFailed(
                        "Notification parameters exceed PostgreSQL's message size limit".into(),
                    ));
                }
                let stmt = client
                    .prepare_typed(NOTIFY, &[Type::TEXT, Type::TEXT])
                    .await
                    .map_err(query_failed)?;
                executing = true;
                let result = client.execute(&stmt, &[&channel, &payload]).await;
                let definitive = match &result {
                    Ok(_) => true,
                    Err(error) => error.as_db_error().is_some_and(|db| {
                        db.parsed_severity() == Some(Severity::Error)
                            && !db.code().code().starts_with("08")
                            && db.code() != &SqlState::T_R_STATEMENT_COMPLETION_UNKNOWN
                    }),
                };
                let result = result.map(|_| ()).map_err(query_failed);
                if definitive {
                    reply = Some(result.clone());
                }
                result
            },
            cancelled,
        )
        .await;
        // A later pool reset or cancellation cannot undo an acknowledged notification.
        reply.unwrap_or_else(|| {
            settled.map_err(|error| {
                if executing {
                    Error::QueryFailed(format!("Notification delivery unconfirmed: {error}"))
                } else {
                    error
                }
            })
        })
    }
}
