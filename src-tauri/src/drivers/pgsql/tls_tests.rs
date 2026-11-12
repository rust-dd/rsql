//! TLS policy against a plaintext server and one with a self-signed certificate.

use tokio_postgres::Config;

use super::commands::pool_connection::{create_pg_pool, full_error_chain};
use super::tls::PgSslMode;

fn config(variable: &str) -> Config {
    let url = std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"));
    url.parse().unwrap()
}

/// Whether the connection the mode produced is encrypted, or the connection error.
async fn encrypted(variable: &str, mode: PgSslMode) -> Result<bool, String> {
    let pool = create_pg_pool(&config(variable), mode, 1).map_err(|e| e.to_string())?;
    let client = pool.get().await.map_err(|e| full_error_chain(&e))?;
    let row = client
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.get(0))
}

const TLS: &str = "RSQL_TEST_TLS_DATABASE_URL";
const PLAIN: &str = "RSQL_TEST_DATABASE_URL";

#[tokio::test]
#[ignore = "requires RSQL_TEST_TLS_DATABASE_URL"]
async fn unverified_modes_encrypt_with_a_self_signed_certificate() {
    for mode in [PgSslMode::Prefer, PgSslMode::Require] {
        assert_eq!(encrypted(TLS, mode).await, Ok(true), "{mode:?}");
    }
    assert_eq!(encrypted(TLS, PgSslMode::Disable).await, Ok(false));
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_TLS_DATABASE_URL"]
async fn verified_modes_reject_an_untrusted_certificate() {
    for mode in [PgSslMode::VerifyCa, PgSslMode::VerifyFull] {
        let error = encrypted(TLS, mode).await.unwrap_err();
        assert!(
            error.to_lowercase().contains("certificate"),
            "{mode:?}: {error}"
        );
    }
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn only_prefer_falls_back_to_plaintext() {
    assert_eq!(encrypted(PLAIN, PgSslMode::Prefer).await, Ok(false));
    assert_eq!(encrypted(PLAIN, PgSslMode::Disable).await, Ok(false));
    for mode in [
        PgSslMode::Require,
        PgSslMode::VerifyCa,
        PgSslMode::VerifyFull,
    ] {
        assert!(encrypted(PLAIN, mode).await.is_err(), "{mode:?}");
    }
}
