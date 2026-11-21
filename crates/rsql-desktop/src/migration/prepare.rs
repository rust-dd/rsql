use super::{
    ImportError, PreparedReceipt, Result, boundary, convert_legacy,
    record::{self, Completion, FORMAT, GENERATION, LIVE, PREPARED, RECEIPT, Record, Source},
    recover,
};
use crate::{connection::Connections, persistence};
use rsql_core::migration::{
    DirectoryId, ProfileLease, open_regular, prepare_database_blocking, read_bundle,
};
use std::{io::Read, path::Path};
use uuid::Uuid;

/// Prepare an empty destination or verify an identical import. All file work runs on a worker.
pub async fn prepare_import(
    bundle: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<PreparedReceipt> {
    prepare(bundle.as_ref(), destination.as_ref(), false).await
}

/// Prepare an explicitly reviewed replacement. The observed profile generation is checked again
/// under an exclusive lease and all existing members are backed up before the first rename.
pub async fn prepare_replacement(
    bundle: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<PreparedReceipt> {
    prepare(bundle.as_ref(), destination.as_ref(), true).await
}

async fn prepare(bundle: &Path, destination: &Path, replace: bool) -> Result<PreparedReceipt> {
    let bundle = bundle.to_owned();
    let destination = destination.to_owned();
    tokio::task::spawn_blocking(move || prepare_closed(&bundle, &destination, replace))
        .await
        .map_err(|_| ImportError::Worker)?
}

pub(super) fn previous(lease: &ProfileLease) -> Result<Option<Record>> {
    previous_at(lease, lease.directory(), &LIVE)
}

/// Validate an installed or archived generation against the same retained activation stage.
pub(super) fn previous_at(
    lease: &ProfileLease,
    folder: &Path,
    names: &[&str; 4],
) -> Result<Option<Record>> {
    let path = folder.join(names[RECEIPT]);
    let generation = folder.join(names[GENERATION]);
    if !record::present(&path)? {
        if record::present(&generation)? {
            return Err(ImportError::Integrity(
                "active receipt missing for current generation",
            ));
        }
        return Ok(None);
    }
    let installed = record::read_record::<Completion>(&path)?.verify(lease)?;
    let expected = installed.installed()?;
    if record::file_hash(&generation)? != expected[GENERATION] {
        return Err(ImportError::Integrity(
            "active receipt does not identify the current generation",
        ));
    }
    if record::file_hash(&path)? != expected[RECEIPT] {
        return Err(ImportError::Integrity("activated receipt bytes"));
    }
    for name in &names[..2] {
        let file = open_regular(&folder.join(name))?;
        if file.metadata()?.len() == 0 {
            return Err(ImportError::Integrity("missing activated profile data"));
        }
    }
    Ok(Some(installed))
}

pub(super) fn reject_reused(
    lease: &ProfileLease,
    mut installed: Record,
    source: &Source,
) -> Result<()> {
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(installed.operation) {
            return Err(ImportError::Integrity("cyclic activation provenance"));
        }
        if installed.source.snapshot_id == source.snapshot_id && installed.source != *source {
            return Err(ImportError::ReusedSnapshot);
        }
        let Some(expected) = installed.old[RECEIPT] else {
            return Ok(());
        };
        let previous = installed.location(lease)?.join(record::BACKUPS[RECEIPT]);
        if record::file_hash(&previous)? != Some(expected) {
            return Err(ImportError::Integrity("previous activation provenance"));
        }
        installed = record::read_record::<Completion>(&previous)?.verify(lease)?;
    }
}

pub(super) fn prepare_closed(
    bundle: &Path,
    destination: &Path,
    replace: bool,
) -> Result<PreparedReceipt> {
    DirectoryId::read(bundle)?;
    let bundle = bundle.canonicalize()?;
    let manifest = read_bundle(&bundle)?;
    let destination = super::destination::create_destination(&bundle, destination)?;
    let mut lease = ProfileLease::acquire(&destination)?;
    let backup_warnings = recover(&mut lease)?.into_backup_warnings();
    prepare_leased(&bundle, manifest, &mut lease, replace, backup_warnings)
}

pub(super) fn prepare_leased(
    bundle: &Path,
    manifest: rsql_core::migration::MigrationManifest,
    lease: &mut ProfileLease,
    replace: bool,
    backup_warnings: Vec<std::path::PathBuf>,
) -> Result<PreparedReceipt> {
    let source = Source::from(&manifest);
    let destination = lease.directory();
    if let Some(installed) = previous(lease)? {
        if installed.source == source {
            return Ok(PreparedReceipt {
                record: installed,
                database: None,
                backup_warnings,
            });
        }
        reject_reused(lease, installed, &source)?;
    }
    super::rollback::reject_reused(lease, &source)?;
    if !replace && other_profile_data(destination)? {
        return Err(ImportError::Conflict);
    }
    let old = record::live_hashes(destination)?;
    record::standalone(destination)?;
    if old[1].is_some() {
        let mut bytes = Vec::new();
        open_regular(&destination.join(LIVE[1]))?.read_to_end(&mut bytes)?;
        persistence::validate_existing(&bytes)?;
    }
    let operation = Uuid::new_v4();
    let stage = record::stage_path(destination, operation);
    check_space(destination, manifest.database_byte_len())?;
    let mut database = prepare_database_blocking(
        bundle,
        manifest.database_byte_len(),
        manifest.database_sha256(),
        &stage,
    )?;
    boundary("migrated database")?;
    let projects = Connections::from_rows(database.project_rows().clone());
    let imported = convert_legacy(manifest.ui(), &projects)?;
    let workspace = persistence::encode_import(&imported.documents)?;
    database.write_import_file(LIVE[1], &workspace)?;
    boundary("workspace synced")?;
    let record = Record {
        version: FORMAT,
        operation,
        replace,
        durable_tabs: imported.documents.tabs.len(),
        discarded_tabs: imported.discarded_tabs,
        directory: lease.verify_directory()?,
        destination: record::bytes_hash(lease.directory().as_os_str().as_encoded_bytes())?,
        stage: DirectoryId::read(&stage)?,
        source,
        database: database.sha256(),
        workspace: record::bytes_hash(&workspace)?,
        old,
        pre_rekey: record::file_hash(&stage.join(rsql_core::migration::PRE_REKEY_BACKUP))?,
    };
    if read_bundle(bundle)? != manifest
        || record::file_hash(&stage.join(LIVE[1]))? != Some(record.workspace)
        || record::file_hash(database.path())? != Some(record.database)
    {
        return Err(ImportError::Integrity(
            "prepared bytes differ from validated input",
        ));
    }
    database.write_import_file(PREPARED, &record::encode(&record)?)?;
    record.validate(lease)?;
    boundary("prepared receipt synced")?;
    Ok(PreparedReceipt {
        record,
        database: Some(database),
        backup_warnings,
    })
}

pub(super) fn other_profile_data(directory: &Path) -> Result<bool> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "rsql.profile.lock" {
            continue;
        }
        if entry.file_type()?.is_dir()
            && name
                .to_str()
                .and_then(|s| s.strip_prefix(".rsql-import-"))
                .is_some_and(|id| Uuid::try_parse(id).is_ok())
        {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

fn check_space(directory: &Path, source_len: u64) -> Result<()> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
        let path =
            CString::new(directory.as_os_str().as_bytes()).map_err(|_| ImportError::Stale)?;
        let mut info = MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: the path is terminated and statvfs initializes the output on success.
        if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } == 0 {
            let info = unsafe { info.assume_init() };
            if u128::from(info.f_bavail) * u128::from(info.f_frsize) < u128::from(source_len) {
                return Err(ImportError::File(std::io::ErrorKind::StorageFull));
            }
        }
    }
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, ptr};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                available: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let path = directory
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut available = 0;
        // SAFETY: the directory is canonical, terminated, and the optional result pointers may be null.
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } != 0
            && available < source_len
        {
            return Err(ImportError::File(std::io::ErrorKind::StorageFull));
        }
    }
    Ok(())
}
