use super::{Candidate, UpdateState, package, source::Source};
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use rsql_core::update::{ARTIFACT_LIMIT, ReleaseCandidate, https_url};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, Write},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub struct VerifiedArtifact {
    candidate: Candidate,
    publisher: PublicKey,
    file: File,
}

struct VerifiedReader(std::io::Take<File>);

impl Read for VerifiedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(bytes)
    }
}

impl VerifiedArtifact {
    pub fn candidate(&self) -> &ReleaseCandidate {
        &self.candidate
    }
    pub fn publisher(&self) -> &PublicKey {
        &self.publisher
    }
    pub fn into_reader(self) -> impl Read {
        VerifiedReader(self.file.take(self.candidate.artifact().bytes))
    }
}

fn decode(encoded: &str) -> Result<String, String> {
    if encoded.len() > 8192 {
        return Err("Publisher record too large.".into());
    }
    String::from_utf8(
        STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid publisher record encoding.")?,
    )
    .map_err(|_| "Invalid publisher record text.".into())
}

pub(super) fn publisher(encoded: &str) -> Result<PublicKey, String> {
    PublicKey::decode(&decode(encoded)?).map_err(|_| "Invalid updater publisher key.".into())
}

pub(super) fn signature(encoded: &str) -> Result<Signature, String> {
    let raw = decode(encoded)?;
    if raw.lines().count() != 4 {
        return Err("Invalid detached signature.".into());
    }
    Signature::decode(&raw).map_err(|_| "Invalid detached signature.".into())
}

pub(super) fn signed_version(sig: &Signature, version: &Version) -> Result<(), String> {
    let mut versions = sig
        .trusted_comment()
        .split('\t')
        .filter_map(|v| v.strip_prefix("version:"));
    if let Some(signed) = versions.next()
        && (Version::parse(signed.strip_prefix('v').unwrap_or(signed))
            .ok()
            .as_ref()
            != Some(version)
            || versions.next().is_some())
    {
        return Err("Signed update version mismatch.".into());
    }
    Ok(())
}

impl Source {
    pub(super) async fn download(
        &self,
        candidate: &Candidate,
        state: &watch::Sender<UpdateState>,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<VerifiedArtifact, String> {
        let deadline = Instant::now() + Duration::from_secs(600);
        let artifact = candidate.artifact();
        let sig = signature(&artifact.signature)?;
        let mut verifier = self
            .key
            .verify_stream(&sig)
            .map_err(|_| "Unsupported artifact signature or publisher.")?;
        let mut file = match &self.stage_parent {
            Some(parent) => tempfile::tempfile_in(parent),
            None => tempfile::tempfile(),
        }
        .map_err(|e| format!("Cannot create private update staging file: {e}"))?;
        #[cfg(unix)]
        if std::os::unix::fs::MetadataExt::nlink(&file.metadata().map_err(|e| e.to_string())?) != 0
        {
            return Err("Cannot secure unnamed update staging file.".into());
        }
        let mut digest = Sha256::new();
        let url = https_url(&artifact.url).map_err(|e| e.to_string())?;
        self.http
            .read(
                &url,
                ARTIFACT_LIMIT,
                Some(artifact.bytes),
                stop,
                |bytes, count, total| {
                    file.write_all(bytes)
                        .map_err(|e| format!("Cannot stage update: {e}"))?;
                    verifier.update(bytes);
                    digest.update(bytes);
                    state.send_if_modified(|s| {
                        if let UpdateState::Downloading {
                            bytes,
                            total: known,
                            ..
                        } = s
                        {
                            *bytes = count;
                            *known = total;
                            true
                        } else {
                            false
                        }
                    });
                    Ok(())
                },
            )
            .await?;
        state.send_if_modified(|s| {
            if matches!(s, UpdateState::Downloading { .. }) {
                *s = UpdateState::Verifying(candidate.clone());
                true
            } else {
                false
            }
        });
        verifier
            .finalize()
            .map_err(|_| "Update artifact publisher signature mismatch.")?;
        if hex::encode(digest.finalize()) != artifact.sha256.to_string() {
            return Err("Update artifact SHA-256 mismatch.".into());
        }
        signed_version(&sig, candidate.version())?;
        file.flush().map_err(|e| e.to_string())?;
        file.rewind().map_err(|e| e.to_string())?;
        package::inspect(&mut file, candidate, stop, deadline)?;
        file.rewind().map_err(|e| e.to_string())?;
        Ok(VerifiedArtifact {
            candidate: candidate.clone(),
            publisher: self.key.clone(),
            file,
        })
    }
}
