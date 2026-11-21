use super::{ACTIVE_LIMIT, BODY_LIMIT, Result, TileKey};
use futures_util::future::BoxFuture;
use reqwest::{
    Client, Response, StatusCode,
    header::{self, HeaderMap},
};
use std::{net::SocketAddr, sync::OnceLock, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct TileProvider(Option<SocketAddr>);

impl TileProvider {
    pub fn openstreetmap() -> Self {
        Self(None)
    }
    pub fn loopback(address: SocketAddr) -> Option<Self> {
        address.ip().is_loopback().then_some(Self(Some(address)))
    }
    pub fn attribution(self) -> (&'static str, &'static str) {
        (
            "© OpenStreetMap contributors",
            "https://www.openstreetmap.org/copyright",
        )
    }
    pub fn url(self, k: TileKey) -> String {
        let base = self.0.map_or_else(
            || "https://tile.openstreetmap.org".into(),
            |addr| format!("http://{addr}"),
        );
        format!("{base}/{}/{}/{}.png", k.z, k.x, k.y)
    }
    pub(super) fn directory(self) -> String {
        self.0.map_or_else(
            || "openstreetmap".into(),
            |addr| format!("loopback-{}", hex::encode(addr.to_string())),
        )
    }
}

/// Abandoning the response future cancels the request. The service reads and limits its body.
pub trait TileTransport: Send + Sync {
    fn request(
        &self,
        provider: TileProvider,
        key: TileKey,
        validators: HeaderMap,
    ) -> BoxFuture<'_, Result<Response>>;
}

#[derive(Default)]
pub(super) struct Http(OnceLock<Result<Client>>);
impl TileTransport for Http {
    fn request(
        &self,
        provider: TileProvider,
        key: TileKey,
        validators: HeaderMap,
    ) -> BoxFuture<'_, Result<Response>> {
        Box::pin(async move {
            let client = self
                .0
                .get_or_init(|| {
                    let client = Client::builder();
                    let client = if provider.0.is_some() {
                        client.no_proxy()
                    } else {
                        client
                    };
                    client
                        .tls_backend_native()
                        .http1_only()
                        .pool_max_idle_per_host(ACTIVE_LIMIT)
                        .redirect(reqwest::redirect::Policy::none())
                        .timeout(Duration::from_secs(15))
                        .user_agent(concat!(
                            "RSQL/",
                            env!("CARGO_PKG_VERSION"),
                            " (+https://github.com/rust-dd/rust-sql)"
                        ))
                        .build()
                        .map_err(|e| e.to_string())
                })
                .as_ref()
                .map_err(Clone::clone)?;
            client
                .get(provider.url(key))
                .header(header::ACCEPT, "image/png")
                .header(header::ACCEPT_ENCODING, "identity")
                .headers(validators)
                .send()
                .await
                .map_err(|e| e.to_string())
        })
    }
}

pub(super) struct HttpFailure {
    pub message: String,
    pub retry: Option<Duration>,
}

impl HttpFailure {
    pub fn network(message: String) -> Self {
        Self {
            message,
            retry: Some(Duration::ZERO),
        }
    }
    fn invalid_body() -> Self {
        Self {
            message: "Tile body exceeds 2 MiB".into(),
            retry: None,
        }
    }
}

pub(super) async fn download(
    transport: &dyn TileTransport,
    provider: TileProvider,
    key: TileKey,
    validators: HeaderMap,
    now: u64,
) -> std::result::Result<(HeaderMap, Option<Vec<u8>>), HttpFailure> {
    let conditional = !validators.is_empty();
    let mut response = transport
        .request(provider, key, validators)
        .await
        .map_err(HttpFailure::network)?;
    let status = response.status();
    let headers = response.headers().clone();
    if status == StatusCode::NOT_MODIFIED && conditional {
        return Ok((headers, None));
    }
    if status != StatusCode::OK {
        let message = format!("Tile provider returned {status}");
        let retry = if status.is_server_error()
            || status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS
        {
            let seconds = headers
                .get(header::RETRY_AFTER)
                .and_then(|v| {
                    let v = v.as_bytes();
                    http_cache_rules::directives::parse_delta_seconds(v).or_else(|| {
                        http_cache_rules::date::parse_http_date(v, now)
                            .map(|date| date.saturating_sub(now))
                    })
                })
                .unwrap_or(0)
                .min(http_cache_rules::directives::DELTA_SECONDS_MAX);
            Some(Duration::from_secs(seconds))
        } else {
            None
        };
        return Err(HttpFailure { message, retry });
    }
    if response
        .content_length()
        .is_some_and(|len| len > BODY_LIMIT as u64)
    {
        return Err(HttpFailure::invalid_body());
    }
    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or(BODY_LIMIT as u64) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| HttpFailure::network(e.to_string()))?
    {
        if chunk.len() > BODY_LIMIT - body.len() {
            return Err(HttpFailure::invalid_body());
        }
        body.extend_from_slice(&chunk);
    }
    Ok((headers, Some(body)))
}
