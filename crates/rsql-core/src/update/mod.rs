//! Pure update descriptions. Publisher verification, live migration authority and
//! installation are separate producer/consumer boundaries.
mod manifest;
mod policy;
mod record;

pub use manifest::*;
pub use policy::*;
pub use record::JsonObject;
