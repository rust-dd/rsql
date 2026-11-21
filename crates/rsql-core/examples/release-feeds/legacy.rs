use super::{Result, asset_name, verify};
use PackageFormat::{AppImage, AppTarGz, Deb, Nsis, Rpm};
use minisign_verify::PublicKey;
use rsql_core::update::{Artifact, JsonObject, NativeManifest, PackageFormat, https_url};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

// Tauri clients try {os}-{arch}-{installer} first, so every Tauri 1.x install falls back to these.
// Deb/rpm keys let package installs upgrade in place; the package name stays `rsql`.
const PLATFORMS: [(&str, &str, PackageFormat); 5] = [
    ("darwin-aarch64", "aarch64-apple-darwin", AppTarGz),
    ("linux-x86_64", "x86_64-unknown-linux-gnu", AppImage),
    ("linux-x86_64-deb", "x86_64-unknown-linux-gnu", Deb),
    ("linux-x86_64-rpm", "x86_64-unknown-linux-gnu", Rpm),
    ("windows-x86_64", "x86_64-pc-windows-msvc", Nsis),
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Feed {
    version: String,
    notes: String,
    pub_date: String,
    platforms: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    signature: String,
    url: String,
}

fn artifact<'a>(
    native: &'a NativeManifest,
    target: &str,
    format: PackageFormat,
) -> Result<&'a Artifact> {
    native
        .artifacts()
        .iter()
        .find(|a| a.target == target && a.format == format)
        .ok_or_else(|| format!("native inventory has no {target} {format:?} artifact").into())
}

pub fn feed(native: &NativeManifest) -> Result<Vec<u8>> {
    let platforms = PLATFORMS
        .iter()
        .map(|&(key, target, format)| {
            let a = artifact(native, target, format)?;
            let entry = Entry {
                signature: a.signature.clone(),
                url: a.url.clone(),
            };
            Ok((key.to_owned(), entry))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let feed = Feed {
        version: native.version().to_string(),
        notes: native.notes().to_owned(),
        pub_date: native.date().to_owned(),
        platforms,
    };
    let mut bytes = serde_json::to_vec_pretty(&feed)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn verify(bytes: &[u8], native: &NativeManifest, dir: &Path, key: &PublicKey) -> Result<()> {
    let JsonObject(feed): JsonObject<Feed> = serde_json::from_slice(bytes)?;
    let version = Version::parse(&feed.version)?;
    chrono::DateTime::parse_from_rfc3339(&feed.pub_date)?;
    if version != *native.version() || feed.platforms.len() != PLATFORMS.len() {
        return Err("legacy feed version or platform set mismatch".into());
    }
    for (platform, target, format) in PLATFORMS {
        let entry = feed
            .platforms
            .get(platform)
            .ok_or("legacy feed is missing a platform")?;
        let a = artifact(native, target, format)?;
        if entry.url != a.url || entry.signature != a.signature {
            return Err("legacy entry differs from the native artifact".into());
        }
        let url = https_url(&entry.url)?;
        let (len, digest) = verify::artifact(
            &dir.join(asset_name(&url)?),
            &entry.signature,
            key,
            &version,
        )?;
        if len != a.bytes || digest != a.sha256.to_string() {
            return Err("legacy length or SHA-256 mismatch".into());
        }
    }
    Ok(())
}
