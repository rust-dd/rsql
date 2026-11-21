use rsql_core::wire::Cell;

const WKT: [&str; 7] = [
    "POINT",
    "LINESTRING",
    "POLYGON",
    "MULTIPOINT",
    "MULTILINESTRING",
    "MULTIPOLYGON",
    "GEOMETRYCOLLECTION",
];
const HINTS: [&str; 10] = [
    "geom",
    "geometry",
    "geography",
    "location",
    "coordinates",
    "the_geom",
    "wkb_geometry",
    "shape",
    "point",
    "latlng",
];

fn source_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000D}'|'\u{0020}'|'\u{00A0}'|'\u{1680}'|'\u{2000}'..='\u{200A}'|'\u{2028}'|'\u{2029}'|'\u{202F}'|'\u{205F}'|'\u{3000}'|'\u{FEFF}')
}

pub(super) fn wkt_prefix(text: &str) -> Option<&'static str> {
    WKT.into_iter().find(|name| {
        text.get(..name.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
            && text[name.len()..]
                .trim_start_matches(source_space)
                .starts_with('(')
    })
}

fn candidate(text: &str) -> bool {
    wkt_prefix(text).is_some()
        || text
            .trim_start_matches(source_space)
            .strip_prefix('{')
            .and_then(|s| s.trim_start_matches(source_space).strip_prefix("\"type\""))
            .is_some_and(|s| s.trim_start_matches(source_space).starts_with(':'))
        || (text.len() >= 40 && text.bytes().all(|c| c.is_ascii_hexdigit()))
}

pub fn detect_column(columns: &[String], rows: &[Vec<Cell>]) -> Option<usize> {
    detect_sample(columns, rows.iter().map(Vec::as_slice))
}

pub fn detect_sample<'a>(
    columns: &[String],
    rows: impl Iterator<Item = &'a [Cell]>,
) -> Option<usize> {
    let sample = rows.take(10).collect::<Vec<_>>();
    if sample.is_empty() {
        return None;
    }
    columns.iter().enumerate().find_map(|(index, column)| {
        let name = column.to_lowercase();
        let required = if HINTS.iter().any(|hint| name.contains(hint)) {
            1
        } else {
            sample.len().min(3)
        };
        let count = sample
            .iter()
            .filter_map(|row| row.get(index)?.as_deref())
            .filter(|value| candidate(value))
            .take(required)
            .count();
        (count == required).then_some(index)
    })
}
