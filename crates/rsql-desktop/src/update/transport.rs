use super::resolver::{RejectLookup, Resolver};
use reqwest::{Client, StatusCode, Url, header};
use rsql_core::update::{METADATA_LIMIT, https_url};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

pub(super) struct Transport {
    pub(super) resolver: Resolver,
}

impl Transport {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            resolver: Resolver::new()?,
        })
    }

    async fn client_for(&self, url: &Url) -> Result<Client, String> {
        let mut builder = Client::builder()
            .http1_only()
            .pool_max_idle_per_host(0)
            .no_proxy()
            .no_gzip()
            .no_deflate()
            .no_brotli()
            .no_zstd()
            .referer(false)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("RSQL-Updater")
            .dns_resolver(Arc::new(RejectLookup));
        if let Some(url::Host::Domain(host)) = url.host() {
            let addresses = self.resolver.resolve(host).await?;
            builder = builder.resolve_to_addrs(host, &addresses);
        }
        builder
            .build()
            .map_err(|_| "Cannot initialize updater transport".into())
    }

    fn validate(&self, url: &Url) -> Result<(), String> {
        https_url(url.as_str())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn redirect(&self, base: &Url, raw: &str) -> Result<Url, String> {
        if raw.is_empty()
            || raw.starts_with("///")
            || raw.bytes().any(|b| b <= 32 || b == 127 || b == b'\\')
        {
            return Err("Invalid update redirect location.".into());
        }
        // URL joining normalizes malformed absolute spellings such as https:///host.
        let url = if Url::parse(raw).is_ok() {
            https_url(raw).map_err(|e| e.to_string())?
        } else {
            base.join(raw).map_err(|_| "Invalid update redirect")?
        };
        self.validate(&url)?;
        Ok(url)
    }

    pub(super) async fn read(
        &self,
        address: &Url,
        limit: u64,
        exact: Option<u64>,
        stop: &mut watch::Receiver<bool>,
        mut consume: impl FnMut(&[u8], u64, Option<u64>) -> Result<(), String>,
    ) -> Result<u64, String> {
        let future = async {
            let mut url = address.clone();
            let mut redirect_budget = METADATA_LIMIT as u64;
            for hop in 0..=5 {
                self.validate(&url)?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                let client = tokio::time::timeout_at(deadline, self.client_for(&url))
                    .await
                    .map_err(|_| "Update hostname lookup timed out.")??;
                let mut response = client
                    .get(url.clone())
                    .timeout(deadline.saturating_duration_since(tokio::time::Instant::now()))
                    .header(header::ACCEPT_ENCODING, "identity")
                    .send()
                    .await
                    .map_err(|_| "The release server could not be reached.")?;
                let status = response.status();
                let redirect = matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308);
                if !redirect && status != StatusCode::OK {
                    return Err(format!("Release server returned HTTP {}.", status.as_u16()));
                }
                if response.headers().contains_key(header::CONTENT_RANGE)
                    || response
                        .headers()
                        .get(header::CONTENT_ENCODING)
                        .is_some_and(|h| h != "identity")
                {
                    return Err("Unexpected partial or encoded update response.".into());
                }
                let declared = response
                    .headers()
                    .get(header::CONTENT_LENGTH)
                    .map(|v| {
                        v.to_str()
                            .ok()
                            .and_then(|v| v.parse::<u64>().ok())
                            .ok_or("Invalid Content-Length")
                    })
                    .transpose()?;
                let cap = if redirect {
                    redirect_budget.min(limit)
                } else {
                    limit.min(exact.unwrap_or(limit))
                };
                if declared.is_some_and(|n| n > cap)
                    || (!redirect && declared.zip(exact).is_some_and(|(a, b)| a != b))
                {
                    return Err(
                        "Update response length exceeds or conflicts with its allowed size.".into(),
                    );
                }
                let next = if redirect {
                    if hop == 5 {
                        return Err("Too many update redirects.".into());
                    }
                    let raw = response
                        .headers()
                        .get(header::LOCATION)
                        .and_then(|h| h.to_str().ok())
                        .ok_or("Missing update redirect location")?;
                    Some(self.redirect(&url, raw)?)
                } else {
                    None
                };
                let mut count = 0;
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| "Update response interrupted.")?
                {
                    if *stop.borrow() {
                        return Err("Update cancelled.".into());
                    }
                    count += chunk.len() as u64;
                    if count > cap || chunk.len() > 1024 * 1024 {
                        return Err("Update response exceeded its byte limit.".into());
                    }
                    if !redirect {
                        consume(&chunk, count, declared)?;
                    }
                }
                if let Some(next) = next {
                    redirect_budget -= count;
                    url = next;
                    continue;
                }
                if exact.or(declared).is_some_and(|n| count != n) {
                    return Err("Incomplete update response.".into());
                }
                return Ok(count);
            }
            Err("Too many redirects.".into())
        };
        let mut cancellation = stop.clone();
        tokio::select! {
            biased;
            _ = cancellation.wait_for(|v| *v) => Err("Update cancelled.".into()),
            result = tokio::time::timeout(Duration::from_secs(600), future) =>
                result.map_err(|_| "Update download timed out.")?,
        }
    }
}
