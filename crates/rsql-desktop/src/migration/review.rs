use super::{Backup, ImportError, Result, prepare, record, rollback};
use rsql_core::migration::{
    DirectoryId, ProfileLease,
    handoff::ExportIdentity,
    legacy::{LegacyKind, decode_legacy_tabs},
    read_bundle,
};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Destination {
    pub path: PathBuf,
    identity: DirectoryId,
}

impl Destination {
    pub fn select(path: &Path) -> Result<Self> {
        let identity = DirectoryId::read(path)?;
        Ok(Self {
            path: path.canonicalize()?,
            identity,
        })
    }

    pub fn same(&self, other: &Self) -> bool {
        self.identity == other.identity
    }

    fn lease(&self) -> Result<ProfileLease> {
        if DirectoryId::read(&self.path)? != self.identity {
            return Err(ImportError::Stale);
        }
        let lease = ProfileLease::acquire(&self.path)?;
        if lease.verify_directory()? != self.identity {
            return Err(ImportError::Stale);
        }
        Ok(lease)
    }
}

enum Observation {
    Closed(record::Hashes),
    Live(Option<uuid::Uuid>),
}

pub struct ImportReview {
    destination: Destination,
    bundle: PathBuf,
    id: DirectoryId,
    source: ExportIdentity,
    manifest: rsql_core::migration::Sha256Digest,
    old: Observation,
    pub conflict: bool,
    pub summary: String,
}

impl ImportReview {
    pub fn inspect(
        bundle: &Path,
        destination: Destination,
        open_profile: Option<&ProfileLease>,
    ) -> Result<Self> {
        let id = DirectoryId::read(bundle)?;
        let bundle = bundle.canonicalize()?;
        let manifest = read_bundle(&bundle)?;
        super::destination::create_destination(&bundle, &destination.path)?;
        let source = ExportIdentity::from(&manifest);
        let manifest_hash = record::bytes_hash(
            &manifest
                .encode()
                .map_err(|_| ImportError::Integrity("manifest encoding"))?,
        )?;
        let live = open_profile
            .map(|lease| lease.verify_directory())
            .transpose()?;
        let (conflict, old) =
            if let Some(lease) = open_profile.filter(|_| live == Some(destination.identity)) {
                let active = prepare::previous(lease)?;
                (
                    !active.as_ref().is_some_and(|r| r.source == source),
                    Observation::Live(active.map(|r| r.operation)),
                )
            } else {
                let lease = destination.lease()?;
                lease.require_resolved_import()?;
                let identical = prepare::previous(&lease)?.is_some_and(|r| r.source == source);
                (
                    !identical && prepare::other_profile_data(lease.directory())?,
                    Observation::Closed(record::live_hashes(lease.directory())?),
                )
            };
        let (mut durable, mut discarded) = (0, 0);
        if let Some(raw) = &manifest.ui().tabs {
            let decoded = decode_legacy_tabs(raw)?;
            discarded = decoded.discarded;
            durable = decoded
                .tabs
                .iter()
                .filter(|t| !matches!(t.fields.kind, LegacyKind::Notify | LegacyKind::Terminal))
                .count();
        }
        let version = manifest
            .source_app_version()
            .chars()
            .take(80)
            .collect::<String>();
        let summary = format!(
            "Format {} · RSQL {version} · snapshot {}\n{} database bytes · {durable} saved tabs · {discarded} malformed tabs",
            manifest.format_version(),
            source.snapshot_id,
            manifest.database_byte_len()
        );
        Ok(Self {
            bundle,
            id,
            destination,
            source,
            manifest: manifest_hash,
            old,
            conflict,
            summary,
        })
    }

    fn activate(self, replace: bool, mut lease: ProfileLease) -> Result<String> {
        if self.conflict && !replace {
            return Err(ImportError::Conflict);
        }
        lease.require_resolved_import()?;
        let bundle = read_bundle(&self.bundle)?;
        if ExportIdentity::from(&bundle) != self.source
            || record::bytes_hash(
                &bundle
                    .encode()
                    .map_err(|_| ImportError::Integrity("manifest encoding"))?,
            )? != self.manifest
            || DirectoryId::read(&self.bundle)? != self.id
        {
            return Err(ImportError::Stale);
        }
        if let Observation::Closed(old) = self.old
            && record::live_hashes(lease.directory())? != old
        {
            return Err(ImportError::Stale);
        }
        let default_query = bundle.ui().tabs.is_none();
        let receipt =
            prepare::prepare_leased(&self.bundle, bundle, &mut lease, replace, Vec::new())?;
        let already_imported = receipt.database.is_none();
        let backup = receipt.record.old[..2]
            .iter()
            .any(Option::is_some)
            .then(|| record::stage_path(lease.directory(), receipt.record.operation));
        let activated = super::activate::activate_closed(receipt, lease)?;
        if already_imported {
            return Ok(format!(
                "This snapshot was already imported into {}. Kept the current native data.",
                self.destination.path.display()
            ));
        }
        let mut message = format!(
            "Imported {} saved tabs into {}.",
            if default_query {
                0
            } else {
                activated.durable_tabs()
            },
            self.destination.path.display()
        );
        if default_query {
            message.push_str(" Opened a fresh Query tab.");
        }
        if let Some(path) = backup {
            message.push_str(&format!(" Previous profile backup: {}", path.display()));
        }
        for path in activated.unverified_backups() {
            message.push_str(&format!(" Unverified backup: {}", path.display()));
        }
        Ok(message)
    }
}

pub enum ReviewedOperation {
    Import {
        review: ImportReview,
        replace: bool,
    },
    Restore {
        destination: Destination,
        backup: Backup,
    },
    Recover(Destination),
}

impl ReviewedOperation {
    fn destination(&self) -> &Destination {
        match self {
            Self::Import { review, .. } => &review.destination,
            Self::Restore { destination, .. } | Self::Recover(destination) => destination,
        }
    }

    pub fn bind_retired(self, lease: &ProfileLease) -> Result<RetiredOperation> {
        if self.destination().identity != lease.verify_directory()? {
            return Err(ImportError::Stale);
        }
        if let Self::Import { review, .. } = &self {
            let generation = prepare::previous(lease)?.map(|r| r.operation);
            if !matches!(review.old, Observation::Live(expected) if expected == generation) {
                return Err(ImportError::Stale);
            }
        }
        Ok(RetiredOperation {
            operation: self,
            drained: record::live_hashes(lease.directory())?,
        })
    }

    pub fn run(self) -> Result<String> {
        if let Self::Import { review, .. } = &self
            && matches!(review.old, Observation::Live(_))
        {
            return Err(ImportError::Stale);
        }
        let lease = self.destination().lease()?;
        self.run_leased(lease)
    }

    fn run_leased(self, mut lease: ProfileLease) -> Result<String> {
        match self {
            Self::Import { review, replace } => review.activate(replace, lease),
            Self::Restore {
                destination,
                backup,
            } => {
                let saved = rollback::restore_backup(backup, lease)?;
                Ok(format!(
                    "Restored {}. Preserved the current profile at {}.",
                    destination.path.display(),
                    saved.display()
                ))
            }
            Self::Recover(destination) => {
                let outcome = super::recover(&mut lease)?;
                Ok(format!(
                    "Recovery completed for {} ({outcome:?}).",
                    destination.path.display()
                ))
            }
        }
    }
}

pub struct RetiredOperation {
    operation: ReviewedOperation,
    drained: record::Hashes,
}

impl RetiredOperation {
    pub fn run(self) -> Result<String> {
        let lease = self.operation.destination().lease()?;
        if record::live_hashes(lease.directory())? != self.drained {
            return Err(ImportError::Stale);
        }
        self.operation.run_leased(lease)
    }
}
