use super::{
    Clock, TileImage, TileKey, TileState,
    cache::{Disk, Record},
    decode::{RasterError, decode},
    policy::Metadata,
    service::Shared,
    transport::{HttpFailure, TileProvider, TileTransport, download},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

pub(super) struct Backend {
    pub provider: TileProvider,
    pub disk: Arc<Disk>,
    pub transport: Arc<dyn TileTransport>,
    pub clock: Clock,
}

pub(super) struct Outcome {
    pub state: TileState,
    pub fresh_until: u64,
    pub retry: Option<Duration>,
    pub offline_until: Option<u64>,
}

impl Outcome {
    pub fn fail(message: String, image: Option<Arc<TileImage>>) -> Self {
        Self {
            state: TileState::Error { message, image },
            fresh_until: 0,
            retry: Some(Duration::ZERO),
            offline_until: None,
        }
    }
    fn ready(image: Arc<TileImage>, fresh_until: u64, disk_error: Option<String>) -> Self {
        let state = match disk_error {
            Some(error) => TileState::Error {
                message: format!("Tile disk cache: {error}"),
                image: Some(image),
            },
            None => TileState::Ready(image),
        };
        Self {
            state,
            fresh_until,
            retry: None,
            offline_until: None,
        }
    }
}

impl Backend {
    async fn raster(
        shared: &Arc<Shared>,
        bytes: Vec<u8>,
        stop: &watch::Receiver<bool>,
    ) -> std::result::Result<(Vec<u8>, Arc<TileImage>), RasterError> {
        let shared = shared.clone();
        let stop = stop.clone();
        tokio::task::spawn_blocking(move || {
            if *stop.borrow() {
                return Err(RasterError::Unavailable("Tile request cancelled".into()));
            }
            let image = decode(&bytes, || shared.reserve())?;
            Ok((bytes, image))
        })
        .await
        .map_err(|e| RasterError::Unavailable(e.to_string()))?
    }

    pub async fn load(
        &self,
        shared: &Arc<Shared>,
        key: TileKey,
        mut stop: watch::Receiver<bool>,
    ) -> Outcome {
        let mut disk_error = None;
        let cached = match self.disk.read(key, stop.clone()).await {
            Ok(value) => value,
            Err(error) => {
                disk_error = Some(error);
                None
            }
        };
        let mut previous = None;
        if let Some(record) = cached {
            match Self::raster(shared, record.bytes, &stop).await {
                Ok((bytes, image)) => {
                    if record.metadata.fresh_until > (self.clock)() {
                        return Outcome::ready(image, record.metadata.fresh_until, disk_error);
                    }
                    previous = Some((
                        Record {
                            metadata: record.metadata,
                            bytes,
                        },
                        image,
                    ));
                }
                Err(RasterError::Unavailable(error)) => return Outcome::fail(error, None),
                Err(RasterError::Invalid(_)) => {
                    disk_error = self.disk.discard(key, stop.clone()).await.err();
                }
            }
        }
        let validators = previous
            .as_ref()
            .map(|(r, _)| r.metadata.validators())
            .unwrap_or_default();
        let sent = (self.clock)();
        let request = download(
            self.transport.as_ref(),
            self.provider,
            key,
            validators,
            sent,
        );
        if *stop.borrow() {
            return Outcome::fail("Tile request cancelled".into(), None);
        }
        let result = tokio::select! {
            biased;
            _ = stop.changed() => return Outcome::fail("Tile request cancelled".into(), None),
            result = tokio::time::timeout(Duration::from_secs(15), request) => {
                result.unwrap_or_else(|_|Err(HttpFailure::network("Tile request timed out".into())))
            }
        };
        let (headers, body) = match result {
            Ok(result) => result,
            Err(HttpFailure {
                message: error,
                retry,
            }) => {
                let offline = previous
                    .filter(|_| retry.is_some())
                    .and_then(|(record, image)| {
                        record
                            .metadata
                            .offline_until
                            .filter(|until| (self.clock)() < *until)
                            .map(|until| (image, until))
                    });
                let (image, offline_until) = offline.unzip();
                let error = match disk_error {
                    Some(disk) => format!("{error}; tile disk cache: {disk}"),
                    None => error,
                };
                let error = if image.is_some() {
                    format!("Offline tile: {error}")
                } else {
                    error
                };
                return Outcome {
                    state: TileState::Error {
                        message: error,
                        image,
                    },
                    retry,
                    offline_until,
                    fresh_until: if retry.is_none() {
                        (self.clock)().saturating_add(60)
                    } else {
                        0
                    },
                };
            }
        };
        let old_meta = body
            .is_none()
            .then(|| &previous.as_ref().unwrap().0.metadata);
        let metadata = match Metadata::response(key, &headers, old_meta, sent, (self.clock)()) {
            Ok(meta) => meta,
            Err(error) => {
                if old_meta.is_some()
                    && let Err(disk) = self.disk.discard(key, stop.clone()).await
                {
                    return Outcome::fail(format!("{error}; tile disk cache: {disk}"), None);
                }
                return Outcome::fail(error, None);
            }
        };
        let (bytes, image) = if let Some(body) = body {
            drop(previous);
            match Self::raster(shared, body, &stop).await {
                Ok(pair) => pair,
                Err(error) => return Outcome::fail(error.message(), None),
            }
        } else {
            let (record, image) = previous.unwrap();
            (record.bytes, image)
        };
        let fresh_until = metadata.fresh_until;
        if let Err(error) = self.disk.store(key, Record { metadata, bytes }, stop).await {
            disk_error = Some(error);
        }
        Outcome::ready(image, fresh_until, disk_error)
    }
}
