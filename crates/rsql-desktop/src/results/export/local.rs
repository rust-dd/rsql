use super::{Format, RowSnapshot, Write, check, chunks, io};
use rsql_core::wire::Cell;
use std::collections::HashSet;

pub(super) fn csv_special(s: &str) -> bool {
    s.contains([',', '"', '\r', '\n'])
}
fn contains(s: &str, test: impl Fn(&str) -> bool, current: &impl Fn() -> bool) -> io::Result<bool> {
    for part in chunks(s) {
        check(current)?;
        if test(part) {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) fn csv_text(
    out: &mut impl Write,
    text: &str,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    let quote = text.is_empty() || contains(text, csv_special, current)?;
    if quote {
        out.write_all(b"\"")?;
    }
    for s in chunks(text) {
        check(current)?;
        if quote {
            out.write_all(s.replace('"', "\"\"").as_bytes())?;
        } else {
            out.write_all(s.as_bytes())?;
        }
    }
    if quote {
        out.write_all(b"\"")?;
    }
    Ok(())
}
fn quoted(
    out: &mut impl Write,
    text: &str,
    mark: char,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    let escape = mark == '\'' && contains(text, |s| s.contains('\\'), current)?;
    if escape {
        out.write_all(b"E")?;
    }
    write!(out, "{mark}")?;
    for part in chunks(text) {
        check(current)?;
        let part = part.replace(mark, if mark == '"' { "\"\"" } else { "''" });
        if escape {
            out.write_all(part.replace('\\', "\\\\").as_bytes())?;
        } else {
            out.write_all(part.as_bytes())?;
        }
    }
    write!(out, "{mark}")
}
fn json(out: &mut impl Write, text: &str, current: &impl Fn() -> bool) -> io::Result<()> {
    out.write_all(b"\"")?;
    for part in chunks(text) {
        check(current)?;
        let quoted = serde_json::to_string(part)?;
        out.write_all(&quoted.as_bytes()[1..quoted.len() - 1])?;
    }
    out.write_all(b"\"")
}
pub(crate) fn xml(
    out: &mut impl Write,
    text: &str,
    attribute: bool,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    for part in chunks(text) {
        check(current)?;
        let mut start = 0;
        for (i, c) in part.char_indices() {
            let replacement = match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\r' => "&#13;",
                '\n' if attribute => "&#10;",
                '\t' if attribute => "&#9;",
                _ => continue,
            };
            out.write_all(&part.as_bytes()[start..i])?;
            out.write_all(replacement.as_bytes())?;
            start = i + c.len_utf8();
        }
        out.write_all(&part.as_bytes()[start..])?;
    }
    Ok(())
}
fn xml_error() -> io::Error {
    io::Error::other(
        "XML cannot represent some control characters in this result. Use JSON or CSV.",
    )
}
fn markdown(out: &mut impl Write, s: &str, current: &impl Fn() -> bool) -> io::Result<()> {
    for part in chunks(s) {
        check(current)?;
        for c in part.chars() {
            match c {
                '&' => out.write_all(b"&amp;")?,
                '<' => out.write_all(b"&lt;")?,
                '>' => out.write_all(b"&gt;")?,
                '|' => out.write_all(b"\\|")?,
                '\\' => out.write_all(b"&#92;&#92;")?,
                '\r' => out.write_all(b"&#92;r")?,
                '\n' => out.write_all(b"&#92;n")?,
                c => out.write_all(c.encode_utf8(&mut [0; 4]).as_bytes())?,
            }
        }
    }
    Ok(())
}
pub(super) struct LocalExport<'a> {
    format: Format,
    snapshot: &'a RowSnapshot,
    table: &'a str,
}

pub(crate) fn valid_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\r' | '\n' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

fn validate_xml(text: &str, current: &impl Fn() -> bool) -> io::Result<()> {
    if contains(text, |s| s.chars().any(|c| !valid_xml_char(c)), current)? {
        return Err(xml_error());
    }
    Ok(())
}

impl<'a> LocalExport<'a> {
    pub(super) fn new(
        format: Format,
        snapshot: &'a RowSnapshot,
        table: &'a str,
        current: &impl Fn() -> bool,
    ) -> io::Result<Self> {
        check(current)?;
        let mut names = HashSet::new();
        for name in snapshot.columns.iter() {
            check(current)?;
            if (format == Format::Json || (format == Format::Sql && !snapshot.is_empty()))
                && !names.insert(name)
            {
                let name = if format == Format::Sql { "SQL" } else { "JSON" };
                return Err(io::Error::other(format!(
                    "{name} export requires unique column names. Add column aliases or use positional CSV or XML."
                )));
            }
            if format == Format::Xml {
                validate_xml(name, current)?;
            }
        }
        for i in 0..snapshot.len() {
            check(current)?;
            let row = snapshot
                .source_index(i)
                .and_then(|index| snapshot.page.get(index))
                .ok_or_else(|| io::Error::other("Unexpected export row index"))?;
            if row.len() != snapshot.columns.len() {
                return Err(io::Error::other("Unexpected export row width"));
            }
            if format == Format::Xml {
                for cell in row.iter() {
                    check(current)?;
                    if let Some(text) = cell {
                        validate_xml(text, current)?;
                    }
                }
            }
        }
        check(current)?;
        Ok(Self {
            format,
            snapshot,
            table,
        })
    }
    pub(super) fn write(self, out: &mut impl Write, current: &impl Fn() -> bool) -> io::Result<()> {
        let rows = (0..self.snapshot.len())
            .map(|i| self.snapshot.page[self.snapshot.source_index(i).unwrap()].as_slice());
        write_local(
            self.format,
            &self.snapshot.columns,
            rows,
            self.table,
            out,
            current,
        )
    }
}

fn write_local<'a>(
    format: Format,
    columns: &[String],
    rows: impl Iterator<Item = &'a [Cell]>,
    table: &str,
    out: &mut impl Write,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    check(current)?;
    match format {
        Format::Csv => {
            for (i, c) in columns.iter().enumerate() {
                check(current)?;
                if i > 0 {
                    out.write_all(b",")?;
                }
                csv_text(out, c, current)?;
            }
            out.write_all(b"\n")?;
        }
        Format::Json => out.write_all(b"[")?,
        Format::Markdown => {
            for separator in [false, true] {
                out.write_all(b"| ")?;
                for (i, c) in columns.iter().enumerate() {
                    check(current)?;
                    if i > 0 {
                        out.write_all(b" | ")?;
                    }
                    markdown(out, if separator { "---" } else { c }, current)?;
                }
                out.write_all(b" |\n")?;
            }
        }
        Format::Xml => {
            out.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<resultset>")?;
        }
        Format::Sql => {}
    }
    let mut empty = true;
    let mut terminate_csv = false;
    for row in rows {
        check(current)?;
        if !empty {
            out.write_all(match format {
                Format::Json => b",",
                Format::Xml => b"",
                _ => b"\n",
            })?;
        }
        empty = false;
        terminate_csv = format == Format::Csv && matches!(row, [] | [None]);
        match format {
            Format::Sql => {
                out.write_all(b"INSERT INTO ")?;
                quoted(out, table, '"', current)?;
                out.write_all(b" (")?;
                for (i, c) in columns.iter().enumerate() {
                    check(current)?;
                    if i > 0 {
                        out.write_all(b", ")?;
                    }
                    quoted(out, c, '"', current)?;
                }
                out.write_all(b") VALUES (")?;
            }
            Format::Markdown => out.write_all(b"| ")?,
            Format::Json => out.write_all(b"\n  {")?,
            Format::Xml => out.write_all(b"\n  <row>")?,
            Format::Csv => {}
        }
        for (i, (column, value)) in columns.iter().zip(row).enumerate() {
            check(current)?;
            if i > 0 {
                out.write_all(match format {
                    Format::Csv | Format::Json => b",",
                    Format::Sql => b", ",
                    Format::Markdown => b" | ",
                    Format::Xml => b"",
                })?;
            }
            match format {
                Format::Csv => {
                    if let Some(s) = value {
                        csv_text(out, s, current)?;
                    }
                }
                Format::Sql => {
                    if let Some(s) = value {
                        quoted(out, s, '\'', current)?;
                    } else {
                        out.write_all(b"NULL")?;
                    }
                }
                Format::Markdown => {
                    if let Some(s) = value {
                        markdown(out, s, current)?;
                    }
                }
                Format::Json => {
                    out.write_all(b"\n    ")?;
                    json(out, column, current)?;
                    out.write_all(b": ")?;
                    if let Some(s) = value {
                        json(out, s, current)?;
                    } else {
                        out.write_all(b"null")?;
                    }
                }
                Format::Xml => {
                    out.write_all(b"\n    <column name=\"")?;
                    xml(out, column, true, current)?;
                    if let Some(s) = value {
                        out.write_all(b"\">")?;
                        xml(out, s, false, current)?;
                        out.write_all(b"</column>")?;
                    } else {
                        out.write_all(b"\" null=\"true\" />")?;
                    }
                }
            }
        }
        out.write_all(match format {
            Format::Csv => b"",
            Format::Json if columns.is_empty() => b"}",
            Format::Json => b"\n  }",
            Format::Sql => b");",
            Format::Markdown => b" |",
            Format::Xml => b"\n  </row>",
        })?;
    }
    check(current)?;
    out.write_all(match format {
        Format::Csv if terminate_csv => b"\n",
        Format::Sql if empty => b"-- No rows to export",
        Format::Json if empty => b"]",
        Format::Json => b"\n]",
        Format::Xml => b"\n</resultset>",
        _ => b"",
    })
}
