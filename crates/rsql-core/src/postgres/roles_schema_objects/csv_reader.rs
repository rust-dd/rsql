use crate::Error;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

struct Checked<R, F>(R, F);

impl<R: Read, F: Fn() -> bool> Read for Checked<R, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !(self.1)() {
            // Interrupted is automatically retried by buffered readers.
            return Err(io::Error::other("CSV read cancelled"));
        }
        self.0.read(buffer)
    }
}

fn open(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| Error::QueryFailed(format!("Failed to read CSV: {e}")))?;
    if !file
        .metadata()
        .map_err(|e| Error::QueryFailed(e.to_string()))?
        .is_file()
    {
        return Err(Error::QueryFailed("Choose a regular CSV file".into()));
    }
    Ok(file)
}

pub type CsvPreview = (Vec<String>, Vec<Vec<String>>);

fn preview(
    reader: impl Read,
    max_rows: usize,
    current: impl Fn() -> bool,
) -> Result<CsvPreview, Error> {
    let fields = |record: &csv::StringRecord| {
        record
            .iter()
            .map(|value| {
                let mut copied = String::with_capacity(value.len());
                let mut rest = value;
                loop {
                    if !current() {
                        return Err(Error::QueryFailed("CSV preview cancelled".into()));
                    }
                    let (part, tail) = rest.split_at(rest.floor_char_boundary(8192));
                    copied.push_str(part);
                    if tail.is_empty() {
                        break;
                    }
                    rest = tail;
                }
                Ok(copied)
            })
            .collect::<Result<Vec<_>, _>>()
    };
    let mut csv = csv::Reader::from_reader(reader);
    let header = fields(
        csv.headers()
            .map_err(|e| Error::QueryFailed(format!("Failed to parse CSV headers: {e}")))?,
    )?;
    let rows = csv
        .records()
        .take(max_rows)
        .map(|r| fields(&r.map_err(|e| Error::QueryFailed(format!("CSV parse error: {e}")))?))
        .collect::<Result<_, _>>()?;
    Ok((header, rows))
}

pub fn parse_csv_preview_checked(
    path: &Path,
    max_rows: usize,
    current: impl Fn() -> bool,
) -> Result<CsvPreview, Error> {
    preview(Checked(open(path)?, &current), max_rows, &current)
}

pub async fn parse_csv_preview(file_path: &str, max_rows: usize) -> Result<CsvPreview, Error> {
    let file = File::open(file_path)
        .map_err(|e| Error::QueryFailed(format!("Failed to read CSV: {e}")))?;
    preview(file, max_rows, || true)
}

/// The explicit end marker distinguishes complete input from a stopped reader.
pub struct CsvRows {
    ready: Option<oneshot::Receiver<Result<(), Error>>>,
    records: mpsc::Receiver<Result<Option<csv::StringRecord>, Error>>,
    pub(super) width: Option<usize>,
}

impl CsvRows {
    pub fn open(path: PathBuf, headers: Option<Arc<Vec<String>>>) -> (Self, JoinHandle<()>) {
        let (send, recv) = mpsc::channel(1);
        let (opened, ready) = oneshot::channel();
        let width = headers.as_ref().map(|h| h.len());
        let worker = tokio::task::spawn_blocking(move || {
            let open_reader = || -> Result<_, Error> {
                let file = if headers.is_some() {
                    open(&path)?
                } else {
                    File::open(&path)
                        .map_err(|e| Error::QueryFailed(format!("Failed to read CSV: {e}")))?
                };
                let mut rdr = csv::Reader::from_reader(Checked(file, || !send.is_closed()));
                let read_header = rdr
                    .headers()
                    .map_err(|e| Error::QueryFailed(format!("Failed to parse CSV headers: {e}")))?;
                if let Some(expected) = headers
                    && !read_header.iter().eq(expected.iter().map(String::as_str))
                {
                    return Err(Error::QueryFailed(
                        "CSV headers changed. Choose the file again.".into(),
                    ));
                }
                Ok(rdr)
            };
            let mut rdr = match open_reader() {
                Ok(reader) => reader,
                Err(e) => {
                    let _ = opened.send(Err(e));
                    return;
                }
            };
            if opened.send(Ok(())).is_err() {
                return;
            }
            let mut read = || -> Result<(), Error> {
                for (i, record) in rdr.records().enumerate() {
                    let row = record.map_err(|e| {
                        Error::QueryFailed(format!("CSV parse error at row {}: {e}", i + 1))
                    })?;
                    if send.blocking_send(Ok(Some(row))).is_err() {
                        return Ok(());
                    }
                }
                Ok(())
            };
            let _ = send.blocking_send(read().map(|()| None));
        });
        (
            Self {
                ready: Some(ready),
                records: recv,
                width,
            },
            worker,
        )
    }
    pub(super) async fn ready(&mut self) -> Result<(), Error> {
        if let Some(ready) = self.ready.take() {
            ready.await.map_err(|_| {
                Error::QueryFailed("CSV reader stopped before opening input".into())
            })??;
        }
        Ok(())
    }

    pub(super) async fn next(&mut self) -> Result<Option<csv::StringRecord>, Error> {
        self.ready().await?;
        self.records
            .recv()
            .await
            .ok_or_else(|| Error::QueryFailed("CSV reader stopped before end of input".into()))?
    }
}
