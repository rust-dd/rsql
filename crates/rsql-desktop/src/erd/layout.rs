use super::{SchemaGraph, TableId};
use icu_collator::Collator;
use icu_locale_core::Locale;
use std::{cmp::Reverse, collections::BTreeMap};

pub const HEADER_HEIGHT: f64 = 32.;
pub const COLUMN_HEIGHT: f64 = 22.;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableBox {
    pub table: TableId,
    pub origin: Point,
    pub width: f64,
    pub height: f64,
}

pub fn layout(graph: &SchemaGraph) -> Vec<TableBox> {
    checked(graph, sys_locale::get_locale().as_deref(), || Ok(()))
        .expect("compiled ICU root collation")
}

pub(super) fn checked(
    graph: &SchemaGraph,
    locale: Option<&str>,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<Vec<TableBox>, String> {
    check()?;
    if graph.tables.is_empty() {
        return Ok(vec![]);
    }
    let locale = locale
        .and_then(|tag| tag.parse::<Locale>().ok())
        .unwrap_or(Locale::UNKNOWN);
    let collator = Collator::try_new(locale.into(), Default::default())
        .or_else(|_| Collator::try_new(Default::default(), Default::default()))
        .map_err(|e| e.to_string())?;
    let ncols = (graph.tables.len() as f64).sqrt().ceil() as usize;
    let mut ordered = BTreeMap::new();
    for (index, table) in graph.tables.iter().enumerate() {
        check()?;
        let id = TableId(table.identity.oid);
        let mut key = Vec::new();
        let Ok(()) = collator.write_sort_key_to(&table.identity.name, &mut key);
        let mut length = table.identity.name.encode_utf16().count();
        for column in &table.columns {
            check()?;
            length = length
                .max(column.name.encode_utf16().count() + 2 + column.ty.encode_utf16().count());
        }
        ordered.insert(
            (Reverse(graph.connections[&id].neighbors.len()), key, index),
            TableBox {
                table: id,
                origin: Point::default(),
                width: (length as f64 * 7. + 40.).max(200.),
                height: HEADER_HEIGHT + table.columns.len() as f64 * COLUMN_HEIGHT + 8.,
            },
        );
    }
    let mut offsets = vec![0_f64; ncols];
    let mut boxes = Vec::with_capacity(ordered.len());
    for (i, b) in ordered.into_values().enumerate() {
        check()?;
        let col = i % ncols;
        offsets[col] = offsets[col].max(b.width);
        boxes.push(b);
    }
    let mut x = 30.;
    for offset in &mut offsets {
        check()?;
        let width = *offset;
        *offset = x;
        x += width + 80.;
    }
    let mut y = 30.;
    for row in boxes.chunks_mut(ncols) {
        let mut height = 0_f64;
        for (b, &x) in row.iter_mut().zip(&offsets) {
            check()?;
            b.origin = Point::new(x, y);
            height = height.max(b.height);
        }
        y += height + 50.;
    }
    Ok(boxes)
}
