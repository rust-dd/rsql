mod executable;
mod metadata;
pub(super) use metadata::bundle;

use flate2::bufread::GzDecoder;
use rsql_core::update::{ARTIFACT_LIMIT, PackageFormat, ReleaseCandidate};
use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader, Read},
    time::Instant,
};
use tokio::sync::watch;

const INFO: &str = "RSQL.app/Contents/Info.plist";
const EXE: &str = "RSQL.app/Contents/MacOS/rsql-desktop";
const HEADER_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy)]
struct Budget<'a> {
    stop: &'a watch::Receiver<bool>,
    deadline: Instant,
}

impl Budget<'_> {
    fn check(self) -> io::Result<()> {
        if *self.stop.borrow() {
            return Err(io::Error::other("Update cancelled"));
        }
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Verification timed out",
            ));
        }
        Ok(())
    }
}

struct Compressed<'a, R> {
    input: R,
    budget: Budget<'a>,
}

impl<R: Read> Read for Compressed<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.budget.check()?;
        let len = out.len().min(8192);
        let n = self.input.read(&mut out[..len])?;
        self.budget.check()?;
        Ok(n)
    }
}

struct Expanded<'a, R: Read> {
    gzip: GzDecoder<BufReader<Compressed<'a, R>>>,
    count: u64,
    budget: Budget<'a>,
}
impl<R: Read> Read for Expanded<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.budget.check()?;
        let n = self.gzip.read(out)?;
        self.count += n as u64;
        if self.count > ARTIFACT_LIMIT {
            return Err(io::Error::other("Expanded archive exceeds limit"));
        }
        Ok(n)
    }
}

pub(super) fn inspect(
    file: impl Read,
    candidate: &ReleaseCandidate,
    stop: &watch::Receiver<bool>,
    deadline: Instant,
) -> Result<(), String> {
    let magic: &[(usize, &[u8])] = match candidate.artifact().format {
        PackageFormat::AppTarGz => {
            return inspect_archive(file, candidate, Budget { stop, deadline })
                .map_err(|e| format!("Invalid RSQL application archive: {e}"));
        }
        PackageFormat::Nsis => &[(0, b"MZ")],
        PackageFormat::Msi => &[(0, b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1")],
        PackageFormat::AppImage => &[(0, b"\x7FELF"), (8, b"AI\x02")],
        _ => {
            return Err(
                "Package identity verification is unavailable for this update format.".into(),
            );
        }
    };
    let mut header = [0; 16];
    file.take(16)
        .read_exact(&mut header)
        .map_err(|e| format!("Invalid update package: {e}"))?;
    if magic
        .iter()
        .any(|&(at, bytes)| &header[at..at + bytes.len()] != bytes)
    {
        return Err("The update package does not match its declared format.".into());
    }
    Ok(())
}

fn inspect_archive(
    file: impl Read,
    candidate: &ReleaseCandidate,
    budget: Budget<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let reader = Expanded {
        gzip: GzDecoder::new(BufReader::new(Compressed {
            input: file,
            budget,
        })),
        count: 0,
        budget,
    };
    let mut archive = tar::Archive::new(reader);
    let mut paths = HashSet::new();
    let mut directories = HashSet::new();
    let mut expected_end = 0;
    let mut has_info = false;
    let mut has_executable = false;
    for entry in archive.entries()?.raw(true) {
        let mut e = entry?;
        let h = e.header();
        let kind = h.entry_type();
        let dir = kind.is_dir();
        if paths.len() >= 4096
            || h.as_ustar().is_none()
            || (!kind.is_file() && !dir)
            || h.link_name_bytes().is_some()
            || (dir && e.size() != 0)
            || e.size() > ARTIFACT_LIMIT
        {
            return Err("unsupported archive entry".into());
        }
        let mode = h.mode()?;
        if mode & !0o777 != 0 {
            return Err("unsupported archive permissions".into());
        }
        let name = h.path_bytes();
        let name = std::str::from_utf8(&name)?;
        let name = if dir {
            name.strip_suffix('/').unwrap_or(name)
        } else {
            name
        };
        if name.len() > 240
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._- /".contains(&b))
            || name
                .split('/')
                .any(|p| matches!(p, "" | "." | "..") || p.ends_with([' ', '.']))
            || !(name == "RSQL.app" || name.starts_with("RSQL.app/"))
        {
            return Err("invalid bundle entry path".into());
        }
        if let Some((parent, _)) = name.rsplit_once('/')
            && !directories.contains(parent)
        {
            return Err("bundle parent directory missing".into());
        }
        if !paths.insert(name.to_ascii_lowercase()) {
            return Err("duplicate package entry".into());
        }
        if dir {
            directories.insert(name.to_owned());
        }
        expected_end = e.raw_file_position() + e.size().next_multiple_of(512);
        match name {
            INFO if !dir && e.size() <= HEADER_LIMIT as u64 => {
                let mut bytes = Vec::with_capacity(e.size() as usize);
                e.read_to_end(&mut bytes)?;
                if bundle(
                    &bytes,
                    &candidate.version().to_string(),
                    candidate.revision(),
                )? != candidate.artifact().minimum_os
                {
                    return Err("bundle OS floor mismatch".into());
                }
                has_info = true;
            }
            EXE if !dir && mode & 0o100 != 0 => {
                let size = e.size();
                executable::inspect(&mut e, size, candidate.artifact())?;
                has_executable = true;
            }
            _ => {}
        }
    }
    let mut reader = archive.into_inner();
    if reader.count != expected_end + 512 {
        return Err("missing archive terminator".into());
    }
    let mut padding = [0; 8192];
    loop {
        let n = reader.read(&mut padding)?;
        if n == 0 {
            break;
        }
        if padding[..n].iter().any(|&v| v != 0) {
            return Err("unexpected data after archive".into());
        }
    }
    if reader.count < expected_end + 1024
        || !reader.count.is_multiple_of(512)
        || !reader.gzip.into_inner().fill_buf()?.is_empty()
    {
        return Err("invalid archive or gzip ending".into());
    }
    if !has_info || !has_executable {
        return Err("missing bundle identity or executable".into());
    }
    budget.check()?;
    Ok(())
}
