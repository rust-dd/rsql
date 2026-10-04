use std::collections::HashMap;
use std::time::{Duration, Instant};

use libsql::{Connection, TransactionBehavior};
use russh::keys::ssh_key::rand_core::{OsRng, RngCore};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::AppState;
use crate::common::enums::AppError;

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

fn database_error(error: libsql::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
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
) -> Result<Option<String>, AppError> {
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
        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
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

    pub async fn approve(&self, id: &str, conn: &Connection) -> Result<(), AppError> {
        let (created, challenge) = self.pending.lock().await.remove(id).ok_or_else(|| {
            AppError::ConnectionFailed("SSH key approval expired. Connect again.".into())
        })?;
        if created.elapsed() >= Duration::from_secs(300) {
            return Err(AppError::ConnectionFailed(
                "SSH key approval expired. Connect again.".into(),
            ));
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(database_error)?;
        let current = known_fingerprint(&tx, &challenge.host, challenge.port).await?;
        if current != challenge.previous_fingerprint {
            return Err(AppError::ConnectionFailed(
                "SSH trust changed while the dialog was open. Connect again.".into(),
            ));
        }
        tx.execute(
            "INSERT INTO ssh_known_hosts (host, port, fingerprint) VALUES (?1, ?2, ?3)
             ON CONFLICT(host, port) DO UPDATE SET fingerprint = excluded.fingerprint",
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

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn ssh_trust_host_key(
    challenge_id: &str,
    state: tauri::State<'_, AppState>,
) -> tauri::Result<()> {
    let conn = state.local_db.connect().map_err(database_error)?;
    state.ssh_host_keys.approve(challenge_id, &conn).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trust_is_explicit_persisted_and_scoped_to_host_and_port() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        initialize(&conn).await.unwrap();
        let approvals = HostKeyApprovals::default();
        let challenge = approvals
            .challenge("SSH.Example", 22, "SHA256:first".into(), None)
            .await;
        assert!(
            known_fingerprint(&conn, "ssh.example", 22)
                .await
                .unwrap()
                .is_none()
        );
        approvals.approve(&challenge.id, &conn).await.unwrap();
        assert_eq!(
            known_fingerprint(&conn, "ssh.example", 22)
                .await
                .unwrap()
                .as_deref(),
            Some("SHA256:first")
        );
        assert!(
            known_fingerprint(&conn, "ssh.example", 2222)
                .await
                .unwrap()
                .is_none()
        );
        assert!(approvals.approve(&challenge.id, &conn).await.is_err());
        let next = approvals
            .challenge(
                "ssh.example",
                22,
                "SHA256:changed".into(),
                Some("SHA256:first".into()),
            )
            .await;
        assert_eq!(
            known_fingerprint(&conn, "ssh.example", 22)
                .await
                .unwrap()
                .as_deref(),
            Some("SHA256:first")
        );
        approvals.approve(&next.id, &conn).await.unwrap();
        assert_eq!(
            known_fingerprint(&conn, "ssh.example", 22)
                .await
                .unwrap()
                .as_deref(),
            Some("SHA256:changed")
        );
    }

    #[tokio::test]
    async fn an_outdated_dialog_cannot_overwrite_newer_trust() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        initialize(&conn).await.unwrap();
        let approvals = HostKeyApprovals::default();
        let first = approvals.challenge("host", 22, "one".into(), None).await;
        let second = approvals.challenge("host", 22, "two".into(), None).await;
        approvals.approve(&first.id, &conn).await.unwrap();
        assert!(approvals.approve(&second.id, &conn).await.is_err());
        assert_eq!(
            known_fingerprint(&conn, "host", 22)
                .await
                .unwrap()
                .as_deref(),
            Some("one")
        );
    }
}
