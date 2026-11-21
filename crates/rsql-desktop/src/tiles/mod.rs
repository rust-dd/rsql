//! Session-scoped raster tiles for visible map viewports.

mod cache;
mod decode;
mod fetch;
mod policy;
mod scheduler;
mod service;
mod transport;

pub use decode::{TileImage, TilePaintLease};
pub use service::{TileConsumer, TileService, TileStats};
pub use transport::{TileProvider, TileTransport};

use std::sync::Arc;

pub const ACTIVE_LIMIT: usize = 4;
pub const QUEUE_LIMIT: usize = 128;
pub const BODY_LIMIT: usize = 2 * 1024 * 1024;
pub const DECODED_LIMIT: usize = 64 * 1024 * 1024;
pub const DISK_LIMIT: u64 = 256 * 1024 * 1024;
const TILE_BYTES: usize = 256 * 256 * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    z: u8,
    x: u32,
    y: u32,
}

impl TileKey {
    pub fn new(z: u8, x: i64, y: i64) -> Option<Self> {
        if z > 19 {
            return None;
        }
        let width = 1i64 << z;
        (0..width).contains(&y).then(|| Self {
            z,
            x: x.rem_euclid(width) as u32,
            y: y as u32,
        })
    }

    pub fn coordinates(self) -> (u8, u32, u32) {
        (self.z, self.x, self.y)
    }
}

#[derive(Default)]
pub struct RetryStatus {
    pub manual: usize,
    pub scheduled: usize,
}

#[derive(Clone, Debug)]
pub enum TileState {
    Loading,
    Ready(Arc<TileImage>),
    Error {
        message: String,
        image: Option<Arc<TileImage>>,
    },
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
type Result<T> = std::result::Result<T, String>;
