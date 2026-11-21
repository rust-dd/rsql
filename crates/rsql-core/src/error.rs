#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("Connection timed out")]
    ConnectionTimeout,
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Project not found: {0}")]
    ProjectNotFound(String),
    #[error("Client not connected: {0}")]
    ClientNotConnected(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Query timed out")]
    QueryTimeout,
    #[error("Query failed: {0}")]
    StatementTimeout(String),
    #[error("Query failed: {0}")]
    StatementCancelled(String),
    #[error("Query failed: {0}")]
    QueryFailed(String),
}

/// Flatten an error and everything it wraps into one message.
///
/// `tokio_postgres::Error` displays as the bare word "db error"; the message
/// the server actually sent — the missing relation, the syntax position, the
/// constraint name — lives in its source. Reporting only the top level gave
/// users "Query failed: db error" for every failure.
pub fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(cause) = src {
        let text = cause.to_string();
        // Some wrappers already embed their source; do not repeat it.
        if !msg.contains(&text) {
            msg.push_str(": ");
            msg.push_str(&text);
        }
        src = cause.source();
    }
    msg
}

/// Convert a Postgres error into a query failure, keeping the server's message.
pub fn query_failed(e: tokio_postgres::Error) -> Error {
    let message = error_chain(&e);
    if let Some(db) = e.as_db_error()
        && db.code() == &tokio_postgres::error::SqlState::QUERY_CANCELED
    {
        return match db.message() {
            "canceling statement due to statement timeout" => Error::StatementTimeout(message),
            // SQLSTATE identifies cancellation even when the primary message is localized.
            _ => Error::StatementCancelled(message),
        };
    }
    Error::QueryFailed(message)
}
