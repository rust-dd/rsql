use super::{
    CHUNK, QueryResult, STOPPED, check, io,
    local::{csv_special, csv_text},
};
use rsql_core::wire;
use std::io::{BufWriter, Write};

fn incomplete() -> io::Error {
    io::Error::other(
        "This result is incomplete or has expired. Run the query again before exporting.",
    )
}
fn cells(
    raw: &str,
    body: bool,
    current: &impl Fn() -> bool,
    mut visit: impl FnMut(&str, bool) -> io::Result<()>,
) -> io::Result<()> {
    let mut error = None;
    let done = wire::visit_cells_checked(raw, body, &mut || current(), |s, end, _| {
        match visit(s, end) {
            Ok(()) => Some(()),
            Err(e) => {
                error = Some(e);
                None
            }
        }
    });
    done.ok_or_else(|| error.unwrap_or_else(|| io::Error::other(STOPPED)))
}
fn packed_field(
    raw: &str,
    header: bool,
    out: &mut impl Write,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    let Some(parts) = wire::cell_chunks(raw) else {
        if header {
            out.write_all(b"\"\"")?;
        }
        return Ok(());
    };
    let mut empty = true;
    let mut quoted = false;
    for part in parts {
        check(current)?;
        empty &= part.is_empty();
        if csv_special(&part) {
            quoted = true;
            break;
        }
    }
    quoted |= empty;
    if quoted {
        out.write_all(b"\"")?;
    }
    for part in wire::cell_chunks(raw).unwrap() {
        check(current)?;
        if quoted && part.contains('"') {
            out.write_all(part.replace('"', "\"\"").as_bytes())?;
        } else {
            out.write_all(part.as_bytes())?;
        }
    }
    if quoted {
        out.write_all(b"\"")?;
    }
    Ok(())
}

pub(super) fn csv_page(
    raw: &str,
    width: usize,
    rows: usize,
    out: &mut impl Write,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    let mut col = 0;
    let mut row = 0;
    cells(raw, true, current, |_, end| {
        col += 1;
        if row >= rows || col > width || (end && col != width) {
            return Err(incomplete());
        }
        if end {
            col = 0;
            row += 1;
        }
        Ok(())
    })?;
    if row != rows {
        return Err(incomplete());
    }
    cells(raw, true, current, |s, end| {
        packed_field(s, false, out, current)?;
        out.write_all(if end { b"\n" } else { b"," })
    })
}

fn wait_for<T>(future: impl Future<Output = T>, current: &impl Fn() -> bool) -> io::Result<T> {
    tokio::runtime::Handle::current().block_on(async {
        tokio::pin!(future);
        loop {
            check(current)?;
            tokio::select! {
                value = &mut future => return Ok(value),
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
            }
        }
    })
}

pub(super) fn retained(
    data: &QueryResult,
    file: std::fs::File,
    current: &impl Fn() -> bool,
    progress: &mut impl FnMut(usize),
) -> io::Result<()> {
    let source = data.packed_source().ok_or_else(incomplete)?;
    let out = CheckedWriter {
        inner: &file,
        current,
    };
    let mut out = BufWriter::with_capacity(CHUNK, out);
    let result = (|| {
        let mut width = 0;
        if let Some(raw) = data.packed_header() {
            cells(raw, false, current, |s, end| {
                packed_field(s, true, &mut out, current)?;
                width += 1;
                if !end {
                    out.write_all(b",")?;
                }
                Ok(())
            })?;
        } else {
            for c in data.columns() {
                check(current)?;
                if width > 0 {
                    out.write_all(b",")?;
                }
                csv_text(&mut out, c, current)?;
                width += 1;
            }
        }
        out.write_all(b"\n")?;
        out.flush()?;
        pages(
            width,
            data.total_rows,
            data.page_size,
            &mut out,
            current,
            progress,
            |offset| {
                wait_for(source.export_page(offset), current)?
                    .map_err(|e| io::Error::other(e.to_string()))
            },
        )
    })();
    // BufWriter's Drop retries pending bytes, even following a failed flush.
    // Leave exactly the bytes already written on cancellation or an I/O error.
    let _ = out.into_parts();
    result?;
    file.sync_all()?;
    if !wait_for(source.available(), current)? {
        return Err(incomplete());
    }
    check(current)
}

struct CheckedWriter<'a, W, F> {
    inner: W,
    current: &'a F,
}
impl<W: Write, F: Fn() -> bool> Write for CheckedWriter<'_, W, F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        check(self.current)?;
        let n = self.inner.write(buf)?;
        if n > buf.len() {
            return Err(io::Error::other("Invalid CSV write count"));
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        check(self.current)?;
        self.inner.flush()
    }
}

fn pages<P: AsRef<str>>(
    width: usize,
    total: usize,
    page_size: usize,
    out: &mut impl Write,
    current: &impl Fn() -> bool,
    progress: &mut impl FnMut(usize),
    mut fetch: impl FnMut(usize) -> io::Result<P>,
) -> io::Result<()> {
    if page_size == 0 {
        return Err(io::Error::other("Invalid result page size"));
    }
    for offset in (0..total).step_by(page_size) {
        check(current)?;
        let page = fetch(offset)?;
        let count = page_size.min(total - offset);
        csv_page(page.as_ref(), width, count, out, current)?;
        out.flush()?;
        progress(offset + count);
    }
    check(current)
}
