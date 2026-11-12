use tokio_postgres::{Client, Error, SimpleQueryMessage, error::SqlState};

use crate::drivers::pgsql::mutation::quote_ident;

#[cfg(test)]
#[path = "session_cleanup_tests.rs"]
mod tests;

// DISCARD ALL without DEALLOCATE ALL, which would invalidate the driver's own
// prepared statements; SQL-level PREPAREs are listed and deallocated instead.
const RESET: &str = "RESET ALL; SET SESSION AUTHORIZATION DEFAULT; CLOSE ALL; UNLISTEN *; \
    SELECT FROM pg_advisory_unlock_all(); DISCARD TEMP; DISCARD SEQUENCES; \
    SELECT name FROM pg_prepared_statements WHERE from_sql";

pub(super) async fn reset_session(client: &Client) -> Result<(), Error> {
    // Keep this in one simple-query batch: its implicit transaction lets LOCAL
    // silence idle ROLLBACK notices.
    let result = client
        .simple_query(&format!(
            "SET LOCAL client_min_messages = error; ROLLBACK; {RESET}"
        ))
        .await;

    let messages = match result {
        // An aborted transaction rejects SET, but is known to need ROLLBACK.
        Err(error) if error.code() == Some(&SqlState::IN_FAILED_SQL_TRANSACTION) => {
            client.simple_query(&format!("ROLLBACK; {RESET}")).await?
        }
        result => result?,
    };

    let deallocate: String = messages
        .iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => row.try_get("name").ok().flatten(),
            _ => None,
        })
        .map(|name| format!("DEALLOCATE {};", quote_ident(name)))
        .collect();
    if !deallocate.is_empty() {
        client.batch_execute(&deallocate).await?;
    }
    Ok(())
}
