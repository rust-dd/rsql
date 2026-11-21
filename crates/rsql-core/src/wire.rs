//! Packed wire format shared by every query path.
//!
//! Cells are joined by [`CELL_SEP`], rows by [`ROW_SEP`]. Because any byte can
//! legitimately appear in a Postgres text value, occurrences of the separators
//! inside data are escaped with [`ESC`] rather than replaced — replacing them
//! silently corrupted values, which broke primary-key matching on row updates.
//! SQL NULL has its own encoding so it stays distinguishable from the text
//! value `"null"` and from the empty string. Empty strings have a marker so a
//! single empty cell cannot be mistaken for an empty page.

/// Cell separator (Unit Separator, ASCII 0x1F).
pub const CELL_SEP: char = '\x1F';
/// Row separator (Record Separator, ASCII 0x1E).
pub const ROW_SEP: char = '\x1E';
/// Escape prefix (Group Separator, ASCII 0x1D).
pub const ESC: char = '\x1D';

const TAG_NULL: char = 'N';
const TAG_EMPTY: char = 'E';
const TAG_CELL_SEP: char = 'A';
const TAG_ROW_SEP: char = 'B';
const TAG_ESC: char = 'C';

/// A single cell: `None` is SQL NULL, `Some("")` is the empty string.
pub type Cell = Option<String>;

pub fn packed_cell_len(cell: Option<&str>) -> usize {
    match cell {
        None | Some("") => 2,
        Some(value) => {
            value.len()
                + value
                    .bytes()
                    .filter(|byte| matches!(byte, 0x1D..=0x1F))
                    .count()
        }
    }
}

/// Append one cell in escaped form. `None` becomes the NULL marker.
pub fn push_cell(out: &mut String, cell: Option<&str>) {
    let Some(value) = cell else {
        out.push(ESC);
        out.push(TAG_NULL);
        return;
    };

    if value.is_empty() {
        out.push(ESC);
        out.push(TAG_EMPTY);
        return;
    }

    // Separators are ASCII, so a byte scan cannot produce false hits inside
    // multi-byte characters and lets the common case skip the escape pass.
    if value.as_bytes().iter().any(|b| matches!(b, 0x1D..=0x1F)) {
        for ch in value.chars() {
            match ch {
                CELL_SEP => {
                    out.push(ESC);
                    out.push(TAG_CELL_SEP);
                }
                ROW_SEP => {
                    out.push(ESC);
                    out.push(TAG_ROW_SEP);
                }
                ESC => {
                    out.push(ESC);
                    out.push(TAG_ESC);
                }
                other => out.push(other),
            }
        }
    } else {
        out.push_str(value);
    }
}

/// Append one row, separating cells with [`CELL_SEP`].
pub fn push_row(out: &mut String, row: &[Cell]) {
    for (index, cell) in row.iter().enumerate() {
        if index > 0 {
            out.push(CELL_SEP);
        }
        push_cell(out, cell.as_deref());
    }
}

/// Capacity estimate for [`pack_rows`] before separator escaping.
fn packed_capacity(rows: &[Vec<Cell>]) -> usize {
    let mut total = 0;
    for row in rows {
        for cell in row {
            total += cell.as_ref().map_or(2, |value| match value.len() {
                0 => 2,
                length => length,
            });
        }
        total += row.len();
    }
    total
}

/// Encode rows into the packed wire format.
pub fn pack_rows(rows: &[Vec<Cell>]) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(packed_capacity(rows));
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            out.push(ROW_SEP);
        }
        push_row(&mut out, row);
    }
    out
}

/// Encode column names as a single header line.
pub fn pack_columns(columns: &[String]) -> String {
    let mut out =
        String::with_capacity(columns.iter().map(String::len).sum::<usize>() + columns.len());
    for (index, name) in columns.iter().enumerate() {
        if index > 0 {
            out.push(CELL_SEP);
        }
        push_cell(&mut out, Some(name));
    }
    out
}

pub fn unpack_cell(raw: &str) -> Cell {
    unpack_cell_checked(raw, &mut || true).expect("uninterrupted decode")
}

const CHECK_BYTES: usize = 8192;

/// Decode with bounded owner checks. Non-marker strings reserve their packed byte length.
pub fn unpack_cell_checked(raw: &str, current: &mut impl FnMut() -> bool) -> Option<Cell> {
    let Some(text) = cell_text(raw) else {
        return Some(None);
    };
    let mut out = String::with_capacity(text.len());
    let mut escaped = false;
    for chunk in chunks(text) {
        if !current() {
            return None;
        }
        if !escaped && !chunk.contains(ESC) {
            out.push_str(chunk);
        } else {
            decode_escapes(chunk, &mut escaped, &mut out);
        }
    }
    Some(Some(out))
}

fn cell_text(raw: &str) -> Option<&str> {
    match raw {
        "\x1dN" => None,
        "\x1dE" => Some(""),
        _ => Some(raw),
    }
}

fn decode_escapes(chunk: &str, escaped: &mut bool, out: &mut String) {
    for ch in chunk.chars() {
        if *escaped {
            out.push(match ch {
                TAG_CELL_SEP => CELL_SEP,
                TAG_ROW_SEP => ROW_SEP,
                TAG_ESC => ESC,
                other => other,
            });
            *escaped = false;
        } else if ch == ESC {
            *escaped = true;
        } else {
            out.push(ch);
        }
    }
}

/// Decode a cell in UTF-8 chunks of at most 8192 bytes. None denotes SQL NULL.
pub fn cell_chunks(raw: &str) -> Option<impl Iterator<Item = std::borrow::Cow<'_, str>>> {
    use std::borrow::Cow;
    let text = cell_text(raw)?;
    let mut escaped = false;
    Some(chunks(text).map(move |chunk| {
        if !escaped && !chunk.contains(ESC) {
            return Cow::Borrowed(chunk);
        }
        let mut out = String::with_capacity(chunk.len());
        decode_escapes(chunk, &mut escaped, &mut out);
        Cow::Owned(out)
    }))
}

fn chunks(mut raw: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if raw.is_empty() {
            return None;
        }
        let (chunk, rest) = raw.split_at(raw.floor_char_boundary(raw.len().min(CHECK_BYTES)));
        raw = rest;
        Some(chunk)
    })
}

/// Visit borrowed cell spans with bounded delimiter scans. The flag marks the last cell of a row.
pub fn visit_cells_checked<F: FnMut() -> bool>(
    raw: &str,
    body: bool,
    current: &mut F,
    mut visit: impl FnMut(&str, bool, &mut F) -> Option<()>,
) -> Option<()> {
    if raw.is_empty() {
        return Some(());
    }
    let mut start = 0;
    let mut offset = 0;
    for chunk in chunks(raw) {
        if !current() {
            return None;
        }
        for (index, sep) in chunk.match_indices([CELL_SEP, ROW_SEP]) {
            if !body && sep == "\x1e" {
                continue;
            }
            let end = offset + index;
            visit(&raw[start..end], sep == "\x1e", current)?;
            start = end + 1;
        }
        offset += chunk.len();
    }
    visit(&raw[start..], true, current)
}

/// Separate the inline-header command fallback from its body without copying either.
pub fn page_parts_checked<'a>(
    header: &'a str,
    packed: &'a str,
    current: &mut impl FnMut() -> bool,
) -> Option<(&'a str, &'a str)> {
    if !header.is_empty() {
        return Some((header, packed));
    }
    let mut offset = 0;
    for chunk in chunks(packed) {
        if !current() {
            return None;
        }
        if let Some(i) = chunk.find(ROW_SEP) {
            return Some((&packed[..offset + i], &packed[offset + i + 1..]));
        }
        offset += chunk.len();
    }
    Some((packed, ""))
}

pub fn unpack_page_checked(
    header: &str,
    packed: &str,
    mut current: impl FnMut() -> bool,
) -> Option<(Vec<String>, Vec<Vec<Cell>>)> {
    let (header, body) = page_parts_checked(header, packed, &mut current)?;
    let mut columns = Vec::new();
    visit_cells_checked(header, false, &mut current, |raw, _, current| {
        columns.push(unpack_cell_checked(raw, current)?.unwrap_or_default());
        Some(())
    })?;
    let mut rows = Vec::new();
    let mut row = Vec::new();
    visit_cells_checked(body, true, &mut current, |raw, end, current| {
        row.push(unpack_cell_checked(raw, current)?);
        if end {
            rows.push(std::mem::take(&mut row));
        }
        Some(())
    })?;
    Some((columns, rows))
}

/// Borrow packed rows and cells for admission checks before allocating decoded data.
pub fn packed_rows(packed: &str) -> impl Iterator<Item = &str> {
    packed.split(ROW_SEP).filter(move |_| !packed.is_empty())
}

pub fn packed_cells(row: &str) -> impl Iterator<Item = &str> {
    row.split(CELL_SEP)
}

pub fn unpack_rows(packed: &str) -> Vec<Vec<Cell>> {
    packed_rows(packed)
        .map(|row| packed_cells(row).map(unpack_cell).collect())
        .collect()
}

pub fn unpack_columns(header: &str) -> Vec<String> {
    if header.is_empty() {
        return Vec::new();
    }
    packed_cells(header)
        .map(|cell| unpack_cell(cell).unwrap_or_default())
        .collect()
}

pub fn split_result(packed: &str) -> (&str, &str) {
    packed.split_once(ROW_SEP).unwrap_or((packed, ""))
}
