use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::Config;
use tokio_postgres::config::SslMode;

use crate::error::Error;

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
    pub fn from_stored(value: &str) -> Result<Self, Error> {
        match value {
            "" | "false" | "disable" => Ok(Self::Disable),
            "prefer" => Ok(Self::Prefer),
            "require" => Ok(Self::Require),
            "verify-ca" => Ok(Self::VerifyCa),
            "true" | "verify-full" => Ok(Self::VerifyFull),
            other => Err(Error::ConnectionFailed(format!(
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
    pub fn connector(self) -> Result<Option<MakeTlsConnector>, Error> {
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
            .map_err(|e| Error::ConnectionFailed(e.to_string()))?;
        Ok(Some(MakeTlsConnector::new(connector)))
    }
}
