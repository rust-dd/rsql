use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::Config;
use tokio_postgres::config::SslMode;

use crate::common::enums::AppError;

/// libpq's sslmode values the app supports; prefer and require encrypt without verifying.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PgSslMode {
    #[default]
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl PgSslMode {
    /// Connections saved before SSL modes stored "true" (a verified certificate) or "false".
    pub fn from_stored(value: &str) -> Result<Self, AppError> {
        match value {
            "" | "false" | "disable" => Ok(Self::Disable),
            "prefer" => Ok(Self::Prefer),
            "require" => Ok(Self::Require),
            "verify-ca" => Ok(Self::VerifyCa),
            "true" | "verify-full" => Ok(Self::VerifyFull),
            other => Err(AppError::ConnectionFailed(format!(
                "Unsupported SSL mode \"{other}\". Edit the connection and choose disable, \
                 prefer, require, verify-ca or verify-full."
            ))),
        }
    }

    pub fn apply(self, cfg: &mut Config) {
        cfg.ssl_mode(match self {
            Self::Disable => SslMode::Disable,
            Self::Prefer => SslMode::Prefer,
            Self::Require | Self::VerifyCa | Self::VerifyFull => SslMode::Require,
        });
    }

    /// `None` means the connection must not use TLS.
    pub fn connector(self) -> Result<Option<MakeTlsConnector>, AppError> {
        let mut builder = TlsConnector::builder();
        match self {
            Self::Disable => return Ok(None),
            Self::Prefer | Self::Require => {
                builder
                    .danger_accept_invalid_certs(true)
                    .danger_accept_invalid_hostnames(true);
            }
            Self::VerifyCa => {
                builder.danger_accept_invalid_hostnames(true);
            }
            Self::VerifyFull => {}
        }
        let connector = builder
            .build()
            .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;
        Ok(Some(MakeTlsConnector::new(connector)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_values_keep_their_meaning() {
        assert_eq!(PgSslMode::from_stored("false").unwrap(), PgSslMode::Disable);
        assert_eq!(PgSslMode::from_stored("").unwrap(), PgSslMode::Disable);
        assert_eq!(
            PgSslMode::from_stored("true").unwrap(),
            PgSslMode::VerifyFull
        );
    }

    #[test]
    fn every_mode_round_trips_and_unknown_modes_are_rejected() {
        for (stored, mode) in [
            ("disable", PgSslMode::Disable),
            ("prefer", PgSslMode::Prefer),
            ("require", PgSslMode::Require),
            ("verify-ca", PgSslMode::VerifyCa),
            ("verify-full", PgSslMode::VerifyFull),
        ] {
            assert_eq!(PgSslMode::from_stored(stored).unwrap(), mode);
            assert_eq!(
                mode.connector().unwrap().is_some(),
                mode != PgSslMode::Disable
            );
        }
        for stored in ["allow", "on", "TRUE"] {
            assert!(PgSslMode::from_stored(stored).is_err(), "{stored}");
        }
    }

    #[test]
    fn modes_that_require_tls_never_fall_back_to_plaintext() {
        for mode in [
            PgSslMode::Require,
            PgSslMode::VerifyCa,
            PgSslMode::VerifyFull,
        ] {
            let mut cfg = Config::new();
            mode.apply(&mut cfg);
            assert_eq!(cfg.get_ssl_mode(), SslMode::Require);
        }
        let mut cfg = Config::new();
        PgSslMode::Disable.apply(&mut cfg);
        assert_eq!(cfg.get_ssl_mode(), SslMode::Disable);
    }
}
