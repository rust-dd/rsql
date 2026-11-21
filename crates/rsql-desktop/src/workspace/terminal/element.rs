use super::{
    TerminalGeometry,
    style::{self, CellStyle},
};
use crate::terminal::{CellFlags, Color, CursorShape, NamedColor, TerminalCell, TerminalSnapshot};
use gpui_kit::*;

pub(super) struct TerminalPaint<'a> {
    pub screen: &'a TerminalSnapshot,
    pub font: Font,
    pub font_size: Pixels,
    pub selection: Hsla,
    pub cursor_shape: CursorShape,
}

impl TerminalPaint<'_> {
    pub fn paint(
        &self,
        geometry: TerminalGeometry,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mask = ContentMask {
            bounds: geometry.content,
        };
        let caret = geometry.cursor_bounds(self.screen, self.cursor_shape);
        window.with_content_mask(Some(mask), |window| {
            geometry.visit(
                self.screen,
                window.content_mask().bounds,
                |row, column, width, cell| {
                    let mut bounds = geometry.cell_bounds(row, column);
                    bounds.size.width *= width as f32;
                    let cursor = &self.screen.cursor;
                    let caret = caret.filter(|_| (cursor.row, cursor.column) == (row, column));
                    self.paint_cell(bounds, cell, caret, window, cx);
                },
            );
        });
        if self.screen.scroll_offset > 0
            && self.screen.history_size > 0
            && !geometry.content.is_empty()
        {
            let track = geometry.content;
            let history = self.screen.history_size as f32;
            let rows = self.screen.rows as f32;
            let height = (track.size.height * (rows / (rows + history)))
                .max(px(8.))
                .min(track.size.height);
            let offset = 1. - self.screen.scroll_offset as f32 / history;
            let thumb = Bounds::new(
                point(
                    bounds.right() - px(5.),
                    track.top() + (track.size.height - height) * offset,
                ),
                size(px(3.), height),
            );
            window.paint_quad(fill(
                thumb,
                style::color(self.screen, Color::Named(NamedColor::Foreground)).opacity(0.5),
            ));
        }
    }

    fn paint_cell(
        &self,
        bounds: Bounds<Pixels>,
        cell: &TerminalCell,
        caret: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut style = CellStyle::new(self.screen, cell, &self.font, self.selection);
        let background = style::background(self.screen);
        let cursor_color = style::color(self.screen, Color::Named(NamedColor::Cursor));
        if caret.is_some() && self.cursor_shape == CursorShape::Block {
            style.foreground = background;
            style.background = cursor_color;
        }
        if style.background != background {
            window.paint_quad(fill(bounds, style.background));
        }
        if !cell.flags.contains(CellFlags::HIDDEN) {
            if cell.text != " " && !cell.flags.contains(CellFlags::LEADING_WIDE_CHAR_SPACER) {
                let run = TextRun {
                    len: cell.text.len(),
                    font: style.font.clone(),
                    color: style.foreground,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    cell.text.clone().into(),
                    self.font_size,
                    &[run],
                    None,
                );
                window.with_content_mask(Some(ContentMask { bounds }), |w| {
                    let _ = line.paint(
                        bounds.origin,
                        bounds.size.height,
                        TextAlign::Left,
                        None,
                        w,
                        cx,
                    );
                });
            }
            decorations(cell.flags, &style, bounds, window);
        }
        if let Some(caret) = caret {
            match self.cursor_shape {
                CursorShape::Beam | CursorShape::Underline => {
                    window.paint_quad(fill(caret, cursor_color))
                }
                CursorShape::HollowBlock => {
                    window.paint_quad(outline(caret, cursor_color, BorderStyle::Solid))
                }
                CursorShape::Block | CursorShape::Hidden => {}
            }
        }
    }
}

fn decorations(flags: CellFlags, style: &CellStyle, bounds: Bounds<Pixels>, window: &mut Window) {
    if flags.intersects(CellFlags::ALL_UNDERLINES) {
        let underline = UnderlineStyle {
            color: Some(style.underline),
            thickness: px(1.),
            wavy: flags.contains(CellFlags::UNDERCURL),
        };
        let origin = point(bounds.left(), bounds.bottom() - px(2.));
        if flags.intersects(CellFlags::DOTTED_UNDERLINE | CellFlags::DASHED_UNDERLINE) {
            let segment = if flags.contains(CellFlags::DOTTED_UNDERLINE) {
                px(1.)
            } else {
                px(3.)
            };
            let mut x = origin.x;
            while x < bounds.right() {
                window.paint_underline(
                    point(x, origin.y),
                    segment.min(bounds.right() - x),
                    &underline,
                );
                x += segment + px(1.);
            }
        } else {
            window.paint_underline(origin, bounds.size.width, &underline);
            if flags.contains(CellFlags::DOUBLE_UNDERLINE) {
                window.paint_underline(
                    origin - point(px(0.), px(2.)),
                    bounds.size.width,
                    &underline,
                );
            }
        }
    }
    if flags.contains(CellFlags::STRIKEOUT) {
        window.paint_strikethrough(
            point(bounds.left(), bounds.center().y),
            bounds.size.width,
            &StrikethroughStyle {
                color: Some(style.foreground),
                thickness: px(1.),
            },
        );
    }
}
