use super::{COLUMN_HEIGHT, EdgeId, HEADER_HEIGHT, Point, SchemaGraph, TableBox, TableId, layout};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub struct DiagramState {
    graph: Arc<SchemaGraph>,
    boxes: Vec<TableBox>,
    index: HashMap<TableId, usize>,
    positions: Vec<Arc<Mutex<Option<Point>>>>,
}

impl DiagramState {
    pub fn new(graph: SchemaGraph) -> Self {
        Self::prepare(graph, None, || Ok(())).expect("compiled ICU root collation")
    }

    pub(super) fn prepare(
        graph: SchemaGraph,
        previous: Option<&Self>,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let boxes = layout::checked(&graph, sys_locale::get_locale().as_deref(), &mut check)?;
        let mut index = HashMap::with_capacity(boxes.len());
        let previous = previous.filter(|old| old.graph.schema() == graph.schema());
        let mut positions = Vec::with_capacity(boxes.len());
        for (i, b) in boxes.iter().enumerate() {
            check()?;
            index.insert(b.table, i);
            let shared =
                previous.and_then(|p| p.index.get(&b.table).map(|&j| p.positions[j].clone()));
            positions.push(shared.unwrap_or_default());
        }
        Ok(Self {
            graph: Arc::new(graph),
            boxes,
            index,
            positions,
        })
    }

    pub fn graph(&self) -> &SchemaGraph {
        &self.graph
    }
    /// Capture each current placement once while the UI owns the drag gesture.
    pub fn export_positions(&self) -> (Arc<SchemaGraph>, Vec<TableBox>) {
        (self.graph.clone(), self.boxes().collect())
    }

    pub fn boxes(&self) -> impl Iterator<Item = TableBox> + '_ {
        self.boxes.iter().map(|b| self.positioned(*b))
    }
    fn positioned(&self, mut b: TableBox) -> TableBox {
        if let Some(pos) = *self.positions[self.index[&b.table]].lock().unwrap() {
            b.origin = pos;
        }
        b
    }

    pub fn table_box(&self, id: TableId) -> Option<TableBox> {
        self.index.get(&id).map(|&i| self.positioned(self.boxes[i]))
    }

    pub fn move_table(&self, id: TableId, to: Point) -> bool {
        let Some(&i) = self.index.get(&id) else {
            return false;
        };
        if !to.x.is_finite() || !to.y.is_finite() {
            return false;
        }
        *self.positions[i].lock().unwrap() = Some(to);
        true
    }

    pub fn geometry(&self, id: EdgeId) -> Option<EdgeGeometry> {
        let edge = self.graph.edges.get(*self.graph.edge_index.get(&id)?)?;
        let source = self.table_box(edge.source.table)?;
        let target = self.table_box(edge.target.table)?;
        self.graph.edge_geometry(edge, source, target)
    }
}

impl SchemaGraph {
    pub(super) fn edge_geometry(
        &self,
        edge: &super::Edge,
        source: TableBox,
        target: TableBox,
    ) -> Option<EdgeGeometry> {
        let source_right = source.origin.x + source.width;
        let target_right = target.origin.x + target.width;
        let (start_x, end_x) = if source_right + 20. < target.origin.x {
            (source_right, target.origin.x)
        } else if target_right + 20. < source.origin.x {
            (source.origin.x, target_right)
        } else {
            (source_right, target_right + 30.)
        };
        let row_center =
            |y: f64, row: usize| y + HEADER_HEIGHT + (row as f64 + 0.5) * COLUMN_HEIGHT;
        let start = Point::new(
            start_x,
            row_center(source.origin.y, *self.column_index.get(&edge.source)?),
        );
        let end = Point::new(
            end_x,
            row_center(target.origin.y, *self.column_index.get(&edge.target)?),
        );
        let offset = 40_f64.max((end_x - start_x).abs() * 0.4);
        Some(EdgeGeometry {
            start,
            end,
            control1: Point::new(start.x + offset, start.y),
            control2: Point::new(end.x - offset, end.y),
            source_radius: 3.,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeGeometry {
    pub start: Point,
    pub control1: Point,
    pub control2: Point,
    pub end: Point,
    pub source_radius: f64,
}

impl EdgeGeometry {
    /// The source SVG uses stroke-width marker units and a (6,4) reference point.
    pub fn arrow(&self, stroke_width: f64) -> [Point; 3] {
        [
            Point::new(
                self.end.x - 6. * stroke_width,
                self.end.y - 4. * stroke_width,
            ),
            Point::new(self.end.x + 2. * stroke_width, self.end.y),
            Point::new(
                self.end.x - 6. * stroke_width,
                self.end.y + 4. * stroke_width,
            ),
        ]
    }
}
