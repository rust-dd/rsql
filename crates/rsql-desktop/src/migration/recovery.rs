use super::{
    ImportError, Result, boundary,
    record::{
        self, BACKUPS, CONFIRMED, FORMAT, GENERATION, Hashes, JOURNAL, LIVE, RECEIPT, Record,
        file_hash,
    },
};
use rsql_core::migration::{ProfileLease, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Clean,
    Installed,
    InstalledWithUnverifiedBackups(Vec<PathBuf>),
    Restored,
}

impl RecoveryOutcome {
    /// Retained backups whose contents could not be verified after the new generation completed.
    pub fn into_backup_warnings(self) -> Vec<PathBuf> {
        match self {
            Self::InstalledWithUnverifiedBackups(paths) => paths,
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
enum Phase {
    Backup,
    Database,
    Workspace,
    Generation,
    Receipt,
    UndoDatabase,
    UndoWorkspace,
    UndoGeneration,
    UndoReceipt,
}

const INSTALL: [Phase; LIVE.len()] = [
    Phase::Database,
    Phase::Workspace,
    Phase::Generation,
    Phase::Receipt,
];
const RESTORE: [Phase; LIVE.len()] = [
    Phase::UndoDatabase,
    Phase::UndoWorkspace,
    Phase::UndoGeneration,
    Phase::UndoReceipt,
];
const INSTALL_BEFORE: [&str; LIVE.len()] = [
    "before database rename",
    "before workspace rename",
    "before generation rename",
    "before receipt rename",
];
const INSTALL_AFTER: [&str; LIVE.len()] = [
    "database renamed",
    "workspace renamed",
    "generation renamed",
    "receipt renamed",
];
const INSTALL_SYNC: [&str; LIVE.len()] = [
    "database synced",
    "workspace installed synced",
    "generation synced",
    "receipt synced",
];
const RESTORE_BEFORE: [&str; LIVE.len()] = [
    "before database restore",
    "before workspace restore",
    "before generation restore",
    "before receipt restore",
];
const RESTORE_AFTER: [&str; LIVE.len()] = [
    "database restored",
    "workspace restored",
    "generation restored",
    "receipt restored",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    version: u32,
    record: Record,
    phase: Phase,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    checksum: Sha256Digest,
    journal: Journal,
}

impl Journal {
    pub fn new(record: Record) -> Self {
        Self {
            version: FORMAT,
            record,
            phase: Phase::Backup,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Borrowed<'a> {
            checksum: Sha256Digest,
            journal: &'a Journal,
        }
        record::encode(&Borrowed {
            checksum: record::bytes_hash(&record::encode(self)?)?,
            journal: self,
        })
    }

    fn persist(&mut self, profile: &Path, stage: &Path, phase: Phase) -> Result<()> {
        self.phase = phase;
        let intent = record::temp_bytes(stage, &self.encode()?)?;
        intent.persist(profile.join(JOURNAL)).map_err(|e| e.error)?;
        record::sync_directory(stage)?;
        record::sync_directory(profile)
    }
}

/// Run synchronously on the pre-Session blocking loader. Exclusive borrowing of the lease
/// is possible only before sharing it with services or after all shared owners are disposed.
pub fn recover(lease: &mut ProfileLease) -> Result<RecoveryOutcome> {
    lease.verify_directory()?;
    let path = lease.directory().join(JOURNAL);
    if !record::present(&path)? {
        super::prepare::previous(lease)?;
        record::sync_directory(lease.directory())?;
        return Ok(RecoveryOutcome::Clean);
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Pending {
        Activation(Envelope),
        Restore(super::rollback::Intent),
    }
    let envelope = match record::read_record::<Pending>(&path)? {
        Pending::Activation(envelope) => envelope,
        Pending::Restore(intent) => return super::rollback::resolve(lease, intent),
    };
    if envelope.journal.version != FORMAT
        || record::bytes_hash(&record::encode(&envelope.journal)?)? != envelope.checksum
    {
        return Err(ImportError::Integrity("journal checksum or version"));
    }
    resolve(lease, envelope.journal)
}

pub(super) fn resolve(lease: &mut ProfileLease, mut journal: Journal) -> Result<RecoveryOutcome> {
    let profile = lease.directory();
    let stage = journal.record.validate(lease)?;
    record::standalone(profile)?;
    record::standalone(&stage)?;
    let old = journal.record.old;
    let new = journal.record.installed()?;
    let live = record::live_hashes(profile)?;
    validate_phase(journal.phase, old, new, live)?;
    if RESTORE.contains(&journal.phase) {
        return rollback(profile, &stage, &mut journal, live);
    }
    let confirmed = file_hash(&stage.join(CONFIRMED))?;
    let mut can_install = if live[RECEIPT] == new[RECEIPT] {
        confirmed == new[RECEIPT]
    } else {
        confirmed.is_none() || confirmed == new[RECEIPT]
    };
    for i in 0..2 {
        if live[i] != new[i] && file_hash(&stage.join(LIVE[i]))? != new[i] {
            can_install = false;
        }
    }
    if !can_install
        || file_hash(&stage.join(rsql_core::migration::PRE_REKEY_BACKUP))?
            != journal.record.pre_rekey
    {
        return rollback(profile, &stage, &mut journal, live);
    }
    if journal.phase == Phase::Backup {
        for i in 0..LIVE.len() {
            if let Some(expected) = old[i] {
                let backup = stage.join(BACKUPS[i]);
                if !record::present(&backup)? {
                    let file = record::temp_copy(&stage, &profile.join(LIVE[i]), expected)?;
                    record::publish_new(file, &backup)?;
                    boundary("backup renamed")?;
                }
            }
        }
        record::sync_directory(&stage)?;
        boundary("backup synced")?;
    }
    let completed = journal.phase == Phase::Receipt && live == new && confirmed == new[RECEIPT];
    let mut backup_warnings = Vec::new();
    for (name, expected) in BACKUPS.iter().zip(old) {
        let path = stage.join(name);
        match file_hash(&path) {
            Ok(actual) if actual == expected => {}
            _ if completed => backup_warnings.push(path),
            Err(error) => return Err(error),
            _ => return rollback(profile, &stage, &mut journal, live),
        }
    }
    let start = INSTALL
        .iter()
        .position(|phase| *phase == journal.phase)
        .unwrap_or_default();
    for i in start..LIVE.len() {
        journal.persist(profile, &stage, INSTALL[i])?;
        if i == RECEIPT && record::live_hashes(profile)?[..RECEIPT] != new[..RECEIPT] {
            return Err(ImportError::Integrity(
                "installed pair differs before receipt publication",
            ));
        }
        let destination = profile.join(LIVE[i]);
        let current = file_hash(&destination)?;
        if current != new[i] {
            if current != old[i] {
                return Err(ImportError::Stale);
            }
            boundary(INSTALL_BEFORE[i])?;
            if i == GENERATION || i == RECEIPT {
                let bytes = if i == RECEIPT {
                    let bytes = record::encode(&journal.record.completion()?)?;
                    let confirmed = stage.join(CONFIRMED);
                    if !record::present(&confirmed)? {
                        record::publish_new(record::temp_bytes(&stage, &bytes)?, &confirmed)?;
                    }
                    if file_hash(&confirmed)? != new[i] {
                        return Err(ImportError::Integrity("activation confirmation"));
                    }
                    record::sync_directory(&stage)?;
                    boundary("confirmation synced")?;
                    bytes
                } else {
                    journal.record.operation.as_bytes().to_vec()
                };
                let candidate = record::temp_bytes(&stage, &bytes)?;
                if old[i].is_none() {
                    record::publish_new(candidate, &destination)?;
                } else {
                    if file_hash(&destination)? != old[i] {
                        return Err(ImportError::Stale);
                    }
                    candidate.persist(&destination).map_err(|e| e.error)?;
                }
            } else {
                if file_hash(&stage.join(LIVE[i]))? != new[i] {
                    return Err(ImportError::Integrity(
                        "staged member changed before rename",
                    ));
                }
                let source = stage.join(LIVE[i]);
                if old[i].is_none() {
                    rsql_core::migration::rename_exclusive(&source, &destination)?;
                } else {
                    if file_hash(&destination)? != old[i] {
                        return Err(ImportError::Stale);
                    }
                    fs::rename(source, &destination)?;
                }
            }
            boundary(INSTALL_AFTER[i])?;
        }
        record::sync_directory(&stage)?;
        record::sync_directory(profile)?;
        boundary(INSTALL_SYNC[i])?;
    }
    if file_hash(&stage.join(CONFIRMED))? != new[RECEIPT] {
        return Err(ImportError::Integrity("activation provenance"));
    }
    for path in &backup_warnings {
        let _ = writeln!(
            std::io::stderr().lock(),
            "Recovered native profile. Previous backup could not be verified: {}",
            path.display()
        );
    }
    finish(profile, new)?;
    if backup_warnings.is_empty() {
        Ok(RecoveryOutcome::Installed)
    } else {
        Ok(RecoveryOutcome::InstalledWithUnverifiedBackups(
            backup_warnings,
        ))
    }
}

fn validate_phase(phase: Phase, old: Hashes, new: Hashes, live: Hashes) -> Result<()> {
    let installing = INSTALL.iter().position(|value| *value == phase);
    let restoring = RESTORE.iter().position(|value| *value == phase);
    for i in 0..LIVE.len() {
        let allowed = match (installing, restoring) {
            (Some(index), _) if i < index => live[i] == new[i],
            (Some(index), _) if i > index => live[i] == old[i],
            (_, Some(index)) if i < index => live[i] == old[i],
            (None, None) => live[i] == old[i],
            _ => live[i] == old[i] || live[i] == new[i],
        };
        if !allowed {
            return Err(ImportError::Integrity(
                "unexpected destination bytes for journal phase",
            ));
        }
    }
    Ok(())
}

fn rollback(
    profile: &Path,
    stage: &Path,
    journal: &mut Journal,
    live: Hashes,
) -> Result<RecoveryOutcome> {
    let old = journal.record.old;
    for i in 0..LIVE.len() {
        if live[i] != old[i] && old[i].is_some() && file_hash(&stage.join(BACKUPS[i]))? != old[i] {
            return Err(ImportError::Integrity(
                "neither a complete import nor a complete previous profile is available",
            ));
        }
    }
    let start = RESTORE
        .iter()
        .position(|phase| *phase == journal.phase)
        .unwrap_or_default();
    for i in start..LIVE.len() {
        journal.persist(profile, stage, RESTORE[i])?;
        let destination = profile.join(LIVE[i]);
        let current = file_hash(&destination)?;
        if current != old[i] {
            if current != journal.record.installed()?[i] {
                return Err(ImportError::Stale);
            }
            boundary(RESTORE_BEFORE[i])?;
            if let Some(digest) = old[i] {
                record::temp_copy(stage, &stage.join(BACKUPS[i]), digest)?
                    .persist(&destination)
                    .map_err(|e| e.error)?;
            } else {
                fs::remove_file(&destination)?;
            }
            boundary(RESTORE_AFTER[i])?;
        }
        record::sync_directory(stage)?;
        record::sync_directory(profile)?;
    }
    finish(profile, old)?;
    Ok(RecoveryOutcome::Restored)
}

pub(super) fn finish(profile: &Path, expected: Hashes) -> Result<()> {
    if record::live_hashes(profile)? != expected {
        return Err(ImportError::Integrity("final import verification"));
    }
    boundary("before journal removal")?;
    fs::remove_file(profile.join(JOURNAL))?;
    boundary("journal removed")?;
    record::sync_directory(profile)
}
