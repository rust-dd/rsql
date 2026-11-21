use std::{ffi::OsString, path::PathBuf};

#[cfg(not(feature = "production"))]
pub const ID: &str = "dev.rsql.native-preview";
#[cfg(not(feature = "production"))]
pub const NAME: &str = "RSQL Native Preview";

#[cfg(feature = "production")]
pub const ID: &str = "com.rust-dd.rsql";
#[cfg(feature = "production")]
pub const NAME: &str = "RSQL";
pub const WEBSITE: &str = "https://github.com/rust-dd/rust-sql";

#[derive(Debug, PartialEq)]
pub enum Error {
    InvalidArguments,
    MissingDataDirectory,
}

pub fn default_root() -> Option<PathBuf> {
    if cfg!(feature = "production") {
        dirs::data_dir()
    } else {
        dirs::data_local_dir()
    }
}

pub fn select(
    mut args: impl Iterator<Item = OsString>,
    default_root: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, Error> {
    let Some(flag) = args.next() else {
        return default_root()
            .map(|root| root.join(ID))
            .ok_or(Error::MissingDataDirectory);
    };
    if flag == "--profile-dir"
        && let Some(path) = args.next()
        && !path.is_empty()
        && !path.to_string_lossy().starts_with("--")
        && args.next().is_none()
    {
        return Ok(path.into());
    }
    Err(Error::InvalidArguments)
}
