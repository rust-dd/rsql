mod destination;
mod local;
mod stream;
use super::{ResultAction, ResultView, diff::RowSnapshot};
use crate::execution::QueryResult;
use local::LocalExport;
use std::{
    io::{self, BufWriter, Read, Seek, Write},
    path::Path,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Csv,
    Json,
    Sql,
    Markdown,
    Xml,
}

const CHUNK: usize = 8192;
const CLIPBOARD_LIMIT: usize = 128 * 1024 * 1024;
const STOPPED: &str = "Export stopped";
fn check(current: &impl Fn() -> bool) -> io::Result<()> {
    if current() {
        Ok(())
    } else {
        Err(io::Error::other(STOPPED))
    }
}
fn chunks(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let (chunk, rest) = text.split_at(text.floor_char_boundary(CHUNK));
        text = rest;
        Some(chunk)
    })
}

pub struct ExportJob {
    pub id: uuid::Uuid,
    pub cancel: tokio::sync::watch::Sender<bool>,
    pub progress: tokio::sync::watch::Receiver<usize>,
    pub retained: bool,
}
pub enum Capture {
    Local(RowSnapshot),
    Packed(Arc<QueryResult>),
}
impl Capture {
    pub fn new(view: &ResultView) -> Option<Self> {
        if !view.allows(ResultAction::Export) {
            return None;
        }
        view.snapshot(ResultAction::Export)
            .map(Self::Local)
            .or_else(|| Self::retained(view))
    }
    pub fn retained(view: &ResultView) -> Option<Self> {
        (view.allows(ResultAction::Export)
            && view.filtered_rows.is_none()
            && view.data.packed_source().is_some())
        .then(|| Self::Packed(view.data.clone()))
    }
    pub fn prepare(
        self,
        format: Format,
        copy: bool,
        current: &impl Fn() -> bool,
    ) -> io::Result<Prepared> {
        self.prepare_with_spool(format, copy, current, tempfile::tempfile)
    }
    fn prepare_with_spool(
        self,
        format: Format,
        copy: bool,
        current: &impl Fn() -> bool,
        spool: impl FnOnce() -> io::Result<std::fs::File>,
    ) -> io::Result<Prepared> {
        check(current)?;
        match self {
            Self::Packed(data) if !copy && format == Format::Csv && data.page_size > 0 => {
                Ok(Prepared::Packed(data))
            }
            Self::Packed(_) => Err(io::Error::other(
                "This result supports saving retained CSV only.",
            )),
            Self::Local(s) => {
                let export = LocalExport::new(format, &s, "table_name", current)?;
                if copy {
                    let mut text = ClipboardBytes(Vec::new());
                    export.write(&mut text, current)?;
                    Ok(Prepared::Clipboard(
                        String::from_utf8(text.0).map_err(io::Error::other)?,
                    ))
                } else {
                    let mut file = spool()?;
                    let mut out = BufWriter::with_capacity(CHUNK, &mut file);
                    export.write(&mut out, current)?;
                    out.flush()?;
                    drop(out);
                    file.rewind()?;
                    Ok(Prepared::Local(file))
                }
            }
        }
    }
}

struct ClipboardBytes(Vec<u8>);
impl Write for ClipboardBytes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let required = self.0.len().saturating_add(buf.len());
        if required > CLIPBOARD_LIMIT {
            return Err(io::Error::other(
                "Export exceeds the 128 MiB clipboard limit. Save to a file.",
            ));
        }
        if required > self.0.capacity() {
            let target = required
                .max(self.0.capacity().saturating_mul(2))
                .clamp(8, CLIPBOARD_LIMIT);
            self.0
                .try_reserve_exact(target - self.0.len())
                .map_err(io::Error::other)?;
        }
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub enum Prepared {
    Clipboard(String),
    Local(std::fs::File),
    Packed(Arc<QueryResult>),
}

fn incomplete_file(path: &Path, error: io::Error) -> io::Error {
    io::Error::other(format!(
        "Export incomplete. {} may contain a partial file: {error}",
        path.display()
    ))
}

/// Replace a user-selected regular file, holding its lock throughout the cancellable copy.
pub fn save_replacing(
    path: &Path,
    mut source: impl Read,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    check(current)?;
    let mut file = destination::replace(path, current)?;
    (|| {
        let mut buf = [0; CHUNK];
        loop {
            check(current)?;
            let n = source.read(&mut buf)?;
            check(current)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
        }
        file.sync_all()
    })()
    .map_err(|e| incomplete_file(path, e))
}

pub(crate) use local::{valid_xml_char, xml};

impl Prepared {
    /// Runs on a blocking worker with an entered Tokio runtime for shared page lookups.
    pub fn save(
        self,
        path: &Path,
        current: &impl Fn() -> bool,
        mut progress: impl FnMut(usize),
    ) -> io::Result<()> {
        check(current)?;
        match self {
            Self::Packed(data) => {
                if data.page_size == 0 {
                    return Err(io::Error::other("Invalid result page size"));
                }
                let file = destination::create_new(path, current)?;
                stream::retained(&data, file, current, &mut progress)
                    .map_err(|e| incomplete_file(path, e))
            }
            Self::Local(spool) => save_replacing(path, spool, current),
            Self::Clipboard(_) => Err(io::Error::other("Expected a file export")),
        }
    }
}
