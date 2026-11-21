use super::{
    COLUMN_HEIGHT, Edge, EdgeGeometry, HEADER_HEIGHT, Point, SchemaGraph, Table, TableBox,
    TableFailure, TableId, table_title,
};
use crate::results::export::{valid_xml_char, xml};
use std::{
    collections::HashMap,
    io::{self, Write},
    sync::Arc,
};

/// Concrete RGB values from the active ERD palette.
#[derive(Clone, Copy)]
pub struct SvgTheme {
    pub background: u32,
    pub card: u32,
    pub border: u32,
    pub primary: u32,
    pub primary_foreground: u32,
    pub foreground: u32,
    pub muted: u32,
    pub muted_foreground: u32,
}

pub fn suggested_filename(schema: &str) -> String {
    // The fixed prefix also keeps Windows device names such as CON and COM1 ordinary.
    let mut name = String::from("erd-");
    for c in schema.chars() {
        if name.len() + c.len_utf8() + 4 > 255 {
            break;
        }
        name.push(if c.is_control() || "<>:\"/\\|?*".contains(c) {
            '_'
        } else {
            c
        });
    }
    name.push_str(".svg");
    name
}

pub type ExportError = io::Error;

pub struct Snapshot {
    pub(super) graph: Arc<SchemaGraph>,
    pub(super) positions: Vec<TableBox>,
    pub(super) failures: Arc<[TableFailure]>,
    pub(super) load_error: Option<String>,
}

impl Snapshot {
    pub fn serialize(
        &self,
        theme: &SvgTheme,
        current: &impl Fn() -> bool,
    ) -> Result<String, ExportError> {
        serialize(
            &self.graph,
            &self.positions,
            theme,
            &self.failures,
            self.load_error.as_deref(),
            current,
        )
    }
}

pub fn export_svg(
    graph: &SchemaGraph,
    positions: &[TableBox],
    theme: &SvgTheme,
) -> Result<String, ExportError> {
    export_svg_checked(graph, positions, theme, &|| true)
}

fn invalid_geometry() -> ExportError {
    io::Error::other("Invalid diagram geometry")
}

struct Extent {
    min: Point,
    max: Point,
}

impl Extent {
    fn include(&mut self, point: Point) -> io::Result<()> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(invalid_geometry());
        }
        self.min.x = self.min.x.min(point.x);
        self.min.y = self.min.y.min(point.y);
        self.max.x = self.max.x.max(point.x);
        self.max.y = self.max.y.max(point.y);
        Ok(())
    }
}

pub fn export_svg_checked(
    graph: &SchemaGraph,
    positions: &[TableBox],
    theme: &SvgTheme,
    current: &impl Fn() -> bool,
) -> Result<String, ExportError> {
    serialize(graph, positions, theme, graph.failures(), None, current)
}

fn serialize(
    graph: &SchemaGraph,
    positions: &[TableBox],
    theme: &SvgTheme,
    failures: &[TableFailure],
    load_error: Option<&str>,
    current: &impl Fn() -> bool,
) -> Result<String, ExportError> {
    let check = || {
        if current() {
            Ok(())
        } else {
            Err(io::Error::other("Export stopped"))
        }
    };
    check()?;
    let mut lookup = HashMap::with_capacity(positions.len());
    let mut extent = Extent {
        min: Point::new(f64::INFINITY, f64::INFINITY),
        max: Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    };
    if positions.len() != graph.tables().len() {
        return Err(invalid_geometry());
    }
    for &b in positions {
        check()?;
        if graph.table(b.table).is_none()
            || lookup.insert(b.table, b).is_some()
            || !(b.width > 0. && b.height > 0.)
        {
            return Err(invalid_geometry());
        }
        extent.include(b.origin)?;
        extent.include(Point::new(b.origin.x + b.width, b.origin.y + b.height))?;
    }
    if positions.is_empty() {
        extent.include(Point::default())?;
    }
    let geometry = |e: &Edge| {
        graph
            .edge_geometry(e, lookup[&e.source.table], lookup[&e.target.table])
            .ok_or_else(invalid_geometry)
    };
    for e in graph.edges() {
        check()?;
        let g = geometry(e)?;
        for p in [g.start, g.end, g.control1, g.control2]
            .into_iter()
            .chain(g.arrow(1.5))
        {
            extent.include(p)?;
        }
    }
    // Enclose the card shadow, source dots and strokes, even for negative manual positions.
    let x = extent.min.x - 30.;
    let warning = !failures.is_empty() || load_error.is_some();
    let y = extent.min.y - 30. - if warning { 32. } else { 0. };
    let width = (extent.max.x - x + 30.).max(if warning { 256. } else { 0. });
    let height = extent.max.y - y + 30.;
    if !width.is_finite() || !height.is_finite() {
        return Err(invalid_geometry());
    }
    let mut out = Vec::new();
    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="{x} {y} {width} {height}" font-family="monospace" xml:space="preserve">"#
    )?;
    out.write_all(b"<title>")?;
    text(&mut out, &graph.schema().name, current)?;
    out.write_all(b"</title>\n")?;
    let SvgTheme {
        primary,
        border,
        background,
        ..
    } = theme;
    writeln!(
        out,
        r##"<defs>
<filter id="erd-shadow" x="-25%" y="-50%" width="150%" height="200%"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#000000" flood-opacity="0.1"/></filter>
<marker id="erd-arrow" markerWidth="8" markerHeight="8" refX="6" refY="4" orient="auto"><path d="M 0 0 L 8 4 L 0 8 Z" fill="#{primary:06x}"/></marker>
<pattern id="erd-grid" width="20" height="20" patternUnits="userSpaceOnUse"><circle cx="10" cy="10" r="0.5" fill="#{border:06x}" opacity="0.5"/></pattern>
</defs>
<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="#{background:06x}"/>
<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="url(#erd-grid)"/>"##
    )?;
    if warning {
        write_warning(
            &mut out,
            x + 12.,
            y + 24.,
            failures,
            load_error,
            theme,
            current,
        )?;
    }
    for edge in graph.edges() {
        check()?;
        write_edge(&mut out, graph, edge, geometry(edge)?, theme, current)?;
    }
    for &b in positions {
        check()?;
        write_table(&mut out, b, graph.table(b.table).unwrap(), theme, current)?;
    }
    out.write_all(b"</svg>\n")?;
    String::from_utf8(out).map_err(io::Error::other)
}

fn text(out: &mut Vec<u8>, value: &str, current: &impl Fn() -> bool) -> io::Result<()> {
    if !value.chars().all(valid_xml_char) {
        return Err(io::Error::other(
            "XML cannot represent a control character in the diagram.",
        ));
    }
    xml(out, value, false, current)
}

fn write_warning(
    out: &mut Vec<u8>,
    x: f64,
    y: f64,
    failures: &[TableFailure],
    load_error: Option<&str>,
    theme: &SvgTheme,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    out.write_all(b"<g><desc>")?;
    if let Some(message) = load_error {
        text(out, message, current)?;
        out.push(b'\n');
    }
    for failure in failures {
        text(out, &failure.table.name, current)?;
        write!(out, " · {:?} · ", failure.section)?;
        text(out, &failure.message, current)?;
        out.push(b'\n');
    }
    writeln!(
        out,
        r##"</desc><text x="{x}" y="{y}" font-size="14" textLength="218" lengthAdjust="spacingAndGlyphs" fill="#{:06x}">Incomplete schema metadata</text></g>"##,
        theme.foreground
    )
}

fn write_edge(
    out: &mut Vec<u8>,
    graph: &SchemaGraph,
    edge: &Edge,
    geometry: EdgeGeometry,
    theme: &SvgTheme,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    let EdgeGeometry {
        start,
        end,
        control1,
        control2,
        source_radius,
    } = geometry;
    let color = theme.muted_foreground;
    let constraint = edge.id.constraint;
    let pair = edge.id.pair;
    write!(
        out,
        r#"<g id="fk-{constraint}-{pair}" opacity="0.7"><title>"#
    )?;
    text(out, &graph.constraint_names[&constraint], current)?;
    out.write_all(b": ")?;
    text(out, &graph.column(edge.source).unwrap().name, current)?;
    out.write_all(" → ".as_bytes())?;
    text(out, &graph.column(edge.target).unwrap().name, current)?;
    writeln!(
        out,
        r##"</title><path d="M {} {} C {} {} {} {} {} {}" fill="none" stroke="#{color:06x}" stroke-width="1.5" marker-end="url(#erd-arrow)"/><circle cx="{}" cy="{}" r="{source_radius}" fill="#{color:06x}"/></g>"##,
        start.x,
        start.y,
        control1.x,
        control1.y,
        control2.x,
        control2.y,
        end.x,
        end.y,
        start.x,
        start.y
    )
}

fn write_table(
    out: &mut Vec<u8>,
    b: TableBox,
    table: &Table,
    theme: &SvgTheme,
    current: &impl Fn() -> bool,
) -> io::Result<()> {
    let TableBox {
        table: TableId(id),
        origin: Point { x, y },
        width,
        height,
    } = b;
    let SvgTheme {
        card,
        primary,
        primary_foreground,
        border,
        muted,
        muted_foreground,
        foreground,
        ..
    } = theme;
    write!(
        out,
        r#"<g id="table-{id}" transform="translate({x} {y})"><title>"#
    )?;
    text(out, &table.identity.name, current)?;
    write!(
        out,
        r##"</title><rect width="{width}" height="{height}" rx="6" fill="#{card:06x}" stroke="#{border:06x}" filter="url(#erd-shadow)"/>
<svg width="{width}" height="{height}" overflow="hidden">
<rect width="{width}" height="{HEADER_HEIGHT}" rx="6" fill="#{primary:06x}"/><rect y="{}" width="{width}" height="6" fill="#{primary:06x}"/>
<line x1="0" y1="{HEADER_HEIGHT}" x2="{width}" y2="{HEADER_HEIGHT}" stroke="#{border:06x}"/>
<text x="10" y="21" font-size="12" font-weight="bold" fill="#{primary_foreground:06x}">"##,
        HEADER_HEIGHT - 6.
    )?;
    text(out, &table_title(&table.identity.name), current)?;
    out.write_all(b"</text>\n")?;
    for (i, column) in table.columns.iter().enumerate() {
        if !current() {
            return Err(io::Error::other("Export stopped"));
        }
        let y = HEADER_HEIGHT + i as f64 * COLUMN_HEIGHT;
        let baseline = y + COLUMN_HEIGHT / 2. + 4.;
        writeln!(out, r#"<g id="column-{id}-{}">"#, column.id.number)?;
        if i % 2 == 1 {
            let radius = if i + 1 == table.columns.len() { 4 } else { 0 };
            writeln!(
                out,
                r##"<rect x="1" y="{y}" width="{}" height="{COLUMN_HEIGHT}" rx="{radius}" fill="#{muted:06x}" opacity="0.3"/>"##,
                width - 2.
            )?;
        }
        let name_x = if let Some(key) = column.key_label() {
            let opacity = if column.primary { 1. } else { 0.7 };
            writeln!(
                out,
                r##"<text x="8" y="{baseline}" font-size="10" font-weight="bold" fill="#{primary:06x}" opacity="{opacity}">{key}</text>"##
            )?;
            28
        } else {
            10
        };
        let (color, weight) = if column.primary {
            (foreground, 600)
        } else {
            (muted_foreground, 400)
        };
        let type_right = (width - 8.).max(0.);
        let name_x = f64::from(name_x).min(type_right);
        let available = (type_right - name_x - 2.).max(0.);
        let type_units = (column.ty.encode_utf16().count() + usize::from(column.nullable)) as f64;
        // Wide fallback glyphs can occupy a full em in the SVG renderer's monospace font.
        let spare_for_type = available - column.name.encode_utf16().count() as f64 * 11.;
        let type_width = (type_units * 10.)
            .min(spare_for_type.max(type_units * 7.))
            .min(available);
        let type_x = type_right - type_width;
        let name_width = available - type_width;
        let text_y = baseline - y;
        write!(
            out,
            r##"<svg x="{name_x}" y="{y}" width="{name_width}" height="{COLUMN_HEIGHT}" overflow="hidden"><text x="0" y="{text_y}" font-size="11" font-weight="{weight}" fill="#{color:06x}">"##
        )?;
        text(out, &column.name, current)?;
        write!(
            out,
            r##"</text></svg><svg x="{type_x}" y="{y}" width="{type_width}" height="{COLUMN_HEIGHT}" overflow="hidden"><text x="{type_width}" y="{text_y}" font-size="10" text-anchor="end" opacity="0.6" fill="#{muted_foreground:06x}">"##
        )?;
        text(out, &column.ty, current)?;
        if column.nullable {
            out.push(b'?');
        }
        out.write_all(b"</text></svg></g>\n")?;
    }
    out.write_all(b"</svg></g>\n")
}
