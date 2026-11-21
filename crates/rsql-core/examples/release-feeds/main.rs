//! Writes latest.json for installed Tauri builds and native-latest.json from NATIVE_DIR's signed
//! inventory and assets, verified against a publisher key supplied independently of that directory.
mod legacy;
mod verify;

use rsql_core::update::{METADATA_LIMIT, NativeManifest, SignedInventory, UpdateError, https_url};
use std::{env, fs, path::Path};
use url::Url;

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

fn asset_name(url: &Url) -> Result<&str> {
    let name = url
        .path_segments()
        .and_then(|mut p| p.next_back())
        .ok_or("missing asset filename")?;
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
    {
        return Err("expected an ASCII release asset filename".into());
    }
    Ok(name)
}

fn generate(key: &Path, native_dir: &Path, out: &Path) -> Result<()> {
    let publisher = verify::publisher(key)?;
    let inventory = verify::bounded_file(&native_dir.join("inventory.json"), METADATA_LIMIT)?;
    let sig = verify::bounded_file(&native_dir.join("inventory.json.sig"), 8192)?;
    let signature = std::str::from_utf8(&sig)?.trim_end();
    let signed_inventory = verify::signature(signature)?;
    publisher.verify(&inventory, &signed_inventory, false)?;
    let native = NativeManifest::parse_inventory(&inventory, |encoded| {
        let invalid = || UpdateError::Invalid("artifact publisher signature or algorithm");
        let sig = verify::signature(encoded).map_err(|_| invalid())?;
        publisher.verify_stream(&sig).map_err(|_| invalid())?;
        Ok(())
    })?;
    verify::signed_version(&signed_inventory, native.version())?;
    for a in native.artifacts() {
        let url = https_url(&a.url)?;
        let filename = asset_name(&url)?;
        let (len, digest) = verify::artifact(
            &native_dir.join(filename),
            &a.signature,
            &publisher,
            native.version(),
        )?;
        if len != a.bytes || digest != a.sha256.to_string() {
            return Err("native length or SHA-256 mismatch".into());
        }
    }
    let envelope = serde_json::to_vec(
        &serde_json::json!({"inventory":std::str::from_utf8(&inventory)?,"signature":signature}),
    )?;
    SignedInventory::parse(&envelope)?;
    let latest = legacy::feed(&native)?;
    legacy::verify(&latest, &native, native_dir, &publisher)?;
    if out.exists() {
        return Err("output directory already exists".into());
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    fs::write(stage.path().join("latest.json"), latest)?;
    fs::write(stage.path().join("native-latest.json"), envelope)?;
    fs::rename(stage.path(), out)?;
    drop(stage.keep());
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let [key, native, out] = args.as_slice() else {
        return Err("usage: release-feeds PUBLISHER_KEY_FILE NATIVE_DIR NEW_OUTPUT_DIR".into());
    };
    generate(Path::new(key), Path::new(native), Path::new(out))
}
