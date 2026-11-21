use crate::terminal::{CellFlags, CursorShape, TerminalCell, TerminalSnapshot};
use gpui_kit::{Bounds, Pixels, Point, Size, point, px, size};

/// Window coordinates in logical pixels. Rows and columns are zero-based in the visible screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalGeometry {
    pub content: Bounds<Pixels>,
    pub cell: Size<Pixels>,
    pub columns: u16,
    pub rows: u16,
}

impl TerminalGeometry {
    pub(super) fn fit(bounds: Bounds<Pixels>, cell: Size<Pixels>) -> Self {
        let available = size(
            (bounds.size.width - px(16.)).max(px(0.)),
            (bounds.size.height - px(8.)).max(px(0.)),
        );
        let columns = (available.width / cell.width).floor().clamp(2., 1024.) as u16;
        let rows = (available.height / cell.height).floor().clamp(1., 256.) as u16;
        let content = Bounds::new(
            bounds.origin + point(px(8.), px(4.)),
            size(
                available.width.min(cell.width * f32::from(columns)),
                available.height.min(cell.height * f32::from(rows)),
            ),
        );
        Self {
            content,
            cell,
            columns,
            rows,
        }
    }

    pub(super) fn clamp_position(&self, position: Point<Pixels>) -> Option<Point<Pixels>> {
        let bounds = self.content;
        if bounds.is_empty() {
            return None;
        }
        Some(point(
            position.x.clamp(
                bounds.left(),
                (bounds.right() - px(0.001)).max(bounds.left()),
            ),
            position.y.clamp(
                bounds.top(),
                (bounds.bottom() - px(0.001)).max(bounds.top()),
            ),
        ))
    }

    pub fn cell_at(&self, point: Point<Pixels>) -> Option<(usize, usize)> {
        self.content.contains(&point).then(|| {
            let local = point - self.content.origin;
            (
                (local.y / self.cell.height) as usize,
                (local.x / self.cell.width) as usize,
            )
        })
    }

    pub fn cell_bounds(&self, row: usize, column: usize) -> Bounds<Pixels> {
        Bounds::new(
            self.content.origin
                + point(
                    self.cell.width * column as f32,
                    self.cell.height * row as f32,
                ),
            self.cell,
        )
    }

    pub(super) fn caret_cell(&self, screen: &TerminalSnapshot) -> Option<Bounds<Pixels>> {
        let c = &screen.cursor;
        (c.row < screen.rows.min(self.rows.into())
            && c.column < screen.columns.min(self.columns.into()))
        .then(|| self.cell_bounds(c.row, c.column))
    }

    pub(super) fn cursor_bounds(
        &self,
        screen: &TerminalSnapshot,
        shape: CursorShape,
    ) -> Option<Bounds<Pixels>> {
        let cursor = &screen.cursor;
        if !cursor.visible {
            return None;
        }
        let mut bounds = self.caret_cell(screen)?;
        if screen.cells[cursor.row * screen.columns + cursor.column]
            .flags
            .contains(CellFlags::WIDE_CHAR)
        {
            bounds.size.width *= 2.;
        }
        match shape {
            CursorShape::Beam => bounds.size.width = px(1.),
            CursorShape::Underline => {
                bounds.origin.y = bounds.bottom() - px(2.);
                bounds.size.height = px(2.);
            }
            CursorShape::Block | CursorShape::HollowBlock => {}
            CursorShape::Hidden => return None,
        }
        Some(bounds)
    }

    pub(super) fn visit(
        &self,
        screen: &TerminalSnapshot,
        clip: Bounds<Pixels>,
        mut paint: impl FnMut(usize, usize, usize, &TerminalCell),
    ) {
        let clip = self.content.intersect(&clip);
        if clip.is_empty() {
            return;
        }
        let start = clip.origin - self.content.origin;
        let end = clip.bottom_right() - self.content.origin;
        let first_col = (start.x / self.cell.width).floor() as usize;
        let columns = screen
            .columns
            .min(usize::from(self.columns))
            .min((end.x / self.cell.width).ceil() as usize);
        let rows = screen
            .rows
            .min(usize::from(self.rows))
            .min((end.y / self.cell.height).ceil() as usize);
        if first_col >= columns {
            return;
        }
        for row in (start.y / self.cell.height).floor() as usize..rows {
            let line = &screen.cells[row * screen.columns..(row + 1) * screen.columns];
            // The leading half of a wide glyph can start just outside the clip.
            let first = if line[first_col].flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                first_col.saturating_sub(1)
            } else {
                first_col
            };
            for (column, cell) in line.iter().enumerate().take(columns).skip(first) {
                if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let width = if cell.flags.contains(CellFlags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
                paint(row, column, width, cell);
            }
        }
    }
}
