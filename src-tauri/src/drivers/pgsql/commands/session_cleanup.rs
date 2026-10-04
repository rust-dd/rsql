use tokio_postgres::{Client, Error, error::SqlState};

pub(super) async fn reset_session(client: &Client) -> Result<(), Error> {
    // Keep this in one simple-query batch: its implicit transaction lets LOCAL
    // silence idle ROLLBACK notices, and ROLLBACK restores the user's setting.
    let result = client
        .batch_execute("SET LOCAL client_min_messages = error; ROLLBACK; RESET statement_timeout")
        .await;

    match result {
        // An aborted transaction rejects SET, but is known to need ROLLBACK.
        Err(error) if error.code() == Some(&SqlState::IN_FAILED_SQL_TRANSACTION) => {
            client
                .batch_execute("ROLLBACK; RESET statement_timeout")
                .await
        }
        result => result,
    }
}
