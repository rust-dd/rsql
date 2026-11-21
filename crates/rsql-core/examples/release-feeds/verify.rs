use super::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use rsql_core::update::ARTIFACT_LIMIT;
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

pub fn bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = regular(path, limit as u64)?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("metadata exceeds limit".into());
    }
    Ok(bytes)
}

fn regular(path: &Path, limit: u64) -> Result<File> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > limit {
        return Err("expected bounded regular file".into());
    }
    Ok(File::open(path)?)
}

fn decoded(encoded: &str) -> Result<String> {
    Ok(String::from_utf8(STANDARD.decode(encoded)?)?)
}

pub fn publisher(path: &Path) -> Result<PublicKey> {
    let key = bounded_file(path, 8192)?;
    Ok(PublicKey::decode(&decoded(
        std::str::from_utf8(&key)?.trim_end(),
    )?)?)
}

pub fn signature(encoded: &str) -> Result<Signature> {
    if encoded.len() > 8192 {
        return Err("signature exceeds limit".into());
    }
    let raw = decoded(encoded)?;
    if raw.lines().count() != 4 {
        return Err("invalid detached signature".into());
    }
    Ok(Signature::decode(&raw)?)
}

pub fn artifact(
    path: &Path,
    encoded: &str,
    key: &PublicKey,
    version: &Version,
) -> Result<(u64, String)> {
    let sig = signature(encoded)?;
    // ED prehashed only. The streaming API also authenticates the trusted comment at finalize.
    let mut verifier = key.verify_stream(&sig)?;
    let mut file = regular(path, ARTIFACT_LIMIT)?;
    let mut sha = Sha256::new();
    let mut count = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > ARTIFACT_LIMIT {
            return Err("artifact exceeds limit".into());
        }
        verifier.update(&buffer[..n]);
        sha.update(&buffer[..n]);
    }
    verifier.finalize()?;
    if count == 0 {
        return Err("empty artifact".into());
    }
    signed_version(&sig, version)?;
    Ok((count, hex::encode(sha.finalize())))
}

pub fn signed_version(sig: &Signature, version: &Version) -> Result<()> {
    let mut signed_versions = sig
        .trusted_comment()
        .split('\t')
        .filter_map(|v| v.strip_prefix("version:"));
    if let Some(signed) = signed_versions.next()
        && (Version::parse(signed.strip_prefix('v').unwrap_or(signed))? != *version
            || signed_versions.next().is_some())
    {
        return Err("signed artifact version mismatch".into());
    }
    Ok(())
}
