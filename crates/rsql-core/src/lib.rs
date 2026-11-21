//! Query behavior and local storage shared by RSQL's desktop frontends.

pub mod diff;
pub mod error;
pub mod metrics;
pub mod migration;
pub mod mutation;
pub mod postgres;
pub mod query;
pub mod services;
mod ssh;
pub use services::ApplicationServices;
#[cfg(feature = "structure")]
pub mod sql;
pub mod storage;
#[cfg(feature = "structure")]
pub mod structure;
pub mod terminal;
pub mod update;
pub mod wire;

pub use error::Error;
