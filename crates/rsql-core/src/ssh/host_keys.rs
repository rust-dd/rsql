use std::collections::HashMap;
use std::time::{Duration, Instant};

use libsql::{Connection, TransactionBehavior};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::error::Error;

pub(crate) const UPSERT_KNOWN_HOST: &str =
    "INSERT INTO ssh_known_hosts (host, port, fingerprint) VALUES (?1, ?2, ?3)
     ON CONFLICT(host, port) DO UPDATE SET fingerprint = excluded.fingerprint";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostKeyChallenge {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
}

#[derive(Default)]
pub(crate) struct HostKeyApprovals {
    pending: Mutex<HashMap<String, (Instant, HostKeyChallenge)>>,
}

fn database_error(error: libsql::Error) -> Error {
    Error::DatabaseError(error.to_string())
}

pub(crate) async fn initialize(conn: &Connection) -> Result<(), libsql::Error> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ssh_known_hosts (
            host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL,
            PRIMARY KEY (host, port)
        )",
        (),
    )
    .await?;
    Ok(())
}

pub(crate) async fn known_fingerprint(
    conn: &Connection,
    host: &str,
    port: u16,
) -> Result<Option<String>, Error> {
    let mut rows = conn
        .query(
            "SELECT fingerprint FROM ssh_known_hosts WHERE host = ?1 AND port = ?2",
            libsql::params![host.to_ascii_lowercase(), i64::from(port)],
        )
        .await
        .map_err(database_error)?;
    rows.next()
        .await
        .map_err(database_error)?
        .map(|row| row.get::<String>(0).map_err(database_error))
        .transpose()
}

impl HostKeyApprovals {
    pub async fn challenge(
        &self,
        host: &str,
        port: u16,
        fingerprint: String,
        previous_fingerprint: Option<String>,
    ) -> HostKeyChallenge {
        let random = rand::random::<[u8; 16]>();
        let id = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let challenge = HostKeyChallenge {
            id: id.clone(),
            host: host.to_ascii_lowercase(),
            port,
            fingerprint,
            previous_fingerprint,
        };
        let mut pending = self.pending.lock().await;
        pending.retain(|_, (created, _)| created.elapsed() < Duration::from_secs(300));
        if pending.len() >= 64 {
            pending.clear();
        }
        pending.insert(id, (Instant::now(), challenge.clone()));
        challenge
    }

    pub async fn approve(&self, id: &str, conn: &Connection) -> Result<(), Error> {
        let (created, challenge) = self.pending.lock().await.remove(id).ok_or_else(|| {
            Error::ConnectionFailed("SSH key approval expired. Connect again.".into())
        })?;
        if created.elapsed() >= Duration::from_secs(300) {
            return Err(Error::ConnectionFailed(
                "SSH key approval expired. Connect again.".into(),
            ));
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(database_error)?;
        let current = known_fingerprint(&tx, &challenge.host, challenge.port).await?;
        if current != challenge.previous_fingerprint {
            return Err(Error::ConnectionFailed(
                "SSH trust changed while the dialog was open. Connect again.".into(),
            ));
        }
        tx.execute(
            UPSERT_KNOWN_HOST,
            libsql::params![
                challenge.host,
                i64::from(challenge.port),
                challenge.fingerprint
            ],
        )
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}
