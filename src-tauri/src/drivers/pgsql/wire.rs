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
pub(crate) const CELL_SEP: char = '\x1F';
/// Row separator (Record Separator, ASCII 0x1E).
pub(crate) const ROW_SEP: char = '\x1E';
/// Escape prefix (Group Separator, ASCII 0x1D).
pub(crate) const ESC: char = '\x1D';

const TAG_NULL: char = 'N';
const TAG_EMPTY: char = 'E';
const TAG_CELL_SEP: char = 'A';
const TAG_ROW_SEP: char = 'B';
const TAG_ESC: char = 'C';

/// A single cell: `None` is SQL NULL, `Some("")` is the empty string.
pub type Cell = Option<String>;

/// Append one cell in escaped form. `None` becomes the NULL marker.
pub(crate) fn push_cell(out: &mut String, cell: Option<&str>) {
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
pub(crate) fn push_row(out: &mut String, row: &[Cell]) {
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
pub(crate) fn pack_rows(rows: &[Vec<Cell>]) -> String {
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
pub(crate) fn pack_columns(columns: &[String]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Fixture {
        name: String,
        rows: Vec<Vec<Cell>>,
        packed: String,
    }

    #[test]
    fn matches_shared_wire_fixtures() {
        let fixtures = serde_json::from_str::<Vec<Fixture>>(include_str!(
            "../../../../tests/fixtures/query-wire.json"
        ))
        .unwrap();
        for fixture in fixtures {
            assert_eq!(pack_rows(&fixture.rows), fixture.packed, "{}", fixture.name);
        }
    }

    /// Mirror of the TypeScript decoder, so round-trips can be asserted here.
    fn unpack_cell(raw: &str) -> Cell {
        if raw == "\x1DN" {
            return None;
        }
        if raw == "\x1DE" {
            return Some(String::new());
        }
        if !raw.contains(ESC) {
            return Some(raw.to_owned());
        }

        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars();
        while let Some(ch) = chars.next() {
            if ch != ESC {
                out.push(ch);
                continue;
            }
            match chars.next() {
                Some(TAG_CELL_SEP) => out.push(CELL_SEP),
                Some(TAG_ROW_SEP) => out.push(ROW_SEP),
                Some(TAG_ESC) => out.push(ESC),
                Some(other) => out.push(other),
                None => {}
            }
        }
        Some(out)
    }

    fn unpack_rows(packed: &str) -> Vec<Vec<Cell>> {
        if packed.is_empty() {
            return Vec::new();
        }
        packed
            .split(ROW_SEP)
            .map(|row| row.split(CELL_SEP).map(unpack_cell).collect())
            .collect()
    }

    fn round_trip(rows: Vec<Vec<Cell>>) {
        let packed = pack_rows(&rows);
        assert_eq!(unpack_rows(&packed), rows);
    }

    #[test]
    fn null_is_distinct_from_the_text_null() {
        let rows = vec![vec![None, Some("null".into()), Some("NULL".into())]];
        let packed = pack_rows(&rows);
        assert_eq!(unpack_rows(&packed), rows);

        let decoded = unpack_rows(&packed);
        assert!(decoded[0][0].is_none());
        assert_eq!(decoded[0][1].as_deref(), Some("null"));
        assert_eq!(decoded[0][2].as_deref(), Some("NULL"));
    }

    #[test]
    fn null_is_distinct_from_the_empty_string() {
        let rows = vec![vec![None, Some(String::new())]];
        let decoded = unpack_rows(&pack_rows(&rows));
        assert!(decoded[0][0].is_none());
        assert_eq!(decoded[0][1].as_deref(), Some(""));
    }

    #[test]
    fn separators_in_data_survive() {
        round_trip(vec![vec![
            Some("a\x1Fb".into()),
            Some("c\x1Ed".into()),
            Some("e\x1Df".into()),
        ]]);
    }

    #[test]
    fn a_value_equal_to_the_null_marker_survives() {
        let rows = vec![vec![Some("\x1DN".into())]];
        let decoded = unpack_rows(&pack_rows(&rows));
        assert_eq!(decoded[0][0].as_deref(), Some("\x1DN"));
    }

    #[test]
    fn consecutive_escapes_survive() {
        round_trip(vec![vec![Some("\x1D\x1D\x1F\x1E".into())]]);
    }

    #[test]
    fn multibyte_content_survives() {
        round_trip(vec![vec![
            Some("árvíztűrő tükörfúrógép".into()),
            Some("日本語".into()),
            Some("emoji 🦀".into()),
        ]]);
    }

    #[test]
    fn multibyte_next_to_separators_survives() {
        round_trip(vec![vec![Some("🦀\x1F🦀\x1E🦀".into())]]);
    }

    #[test]
    fn empty_input_packs_to_empty_string() {
        assert_eq!(pack_rows(&[]), "");
        assert!(unpack_rows("").is_empty());
    }

    #[test]
    fn multiple_rows_and_columns_survive() {
        round_trip(vec![
            vec![Some("1".into()), None, Some("x".into())],
            vec![None, Some(String::new()), Some("null".into())],
            vec![Some("3".into()), Some("y".into()), None],
        ]);
    }

    #[test]
    fn column_names_are_escaped_too() {
        let columns = vec!["id".to_string(), "we\x1Fird".to_string()];
        let packed = pack_columns(&columns);
        let decoded = packed.split(CELL_SEP).map(unpack_cell).collect::<Vec<_>>();
        assert_eq!(decoded[0].as_deref(), Some("id"));
        assert_eq!(decoded[1].as_deref(), Some("we\x1Fird"));
    }

    #[test]
    fn capacity_estimate_covers_unescaped_payloads() {
        let rows = vec![
            vec![Some("abc".into()), None],
            vec![Some("de".into()), Some("f".into())],
        ];
        assert!(packed_capacity(&rows) >= pack_rows(&rows).len());
    }
}
