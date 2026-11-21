// RSQL local source changes. See NOTICE for upstream provenance and modifications.
//! A visual row whose offsets remain in source UTF-8 bytes.
use gpui::{App, Pixels, Point, ShapedLine, SharedString, TextAlign, Window, point, px};
use std::ops::Range;

pub(crate) struct InlineFragment {
    pub(crate) range: Range<usize>,
    pub(crate) x: Pixels,
    pub(crate) width: Pixels,
    pub(crate) text: Option<ShapedLine>,
}

pub(crate) struct InputLine {
    pub(crate) len: usize,
    pub(crate) width: Pixels,
    pub(crate) text: SharedString,
    content: Content,
}
// Keep the ordinary shaped row inline: boxing it would add an allocation to
// every existing plain-text row. Only token rows allocate fragment storage.
#[allow(clippy::large_enum_variant)]
enum Content {
    Text(ShapedLine),
    Projected(ShapedLine, Range<usize>, Range<usize>, Pixels),
    Inline(Vec<InlineFragment>),
}
impl From<ShapedLine> for InputLine {
    fn from(line: ShapedLine) -> Self {
        Self {
            len: line.len,
            width: line.width,
            text: line.text.clone(),
            content: Content::Text(line),
        }
    }
}
impl InputLine {
    pub(crate) fn projected(
        source: SharedString,
        shape: ShapedLine,
        hidden: Range<usize>,
        ghost: Range<usize>,
        indent: Pixels,
    ) -> Self {
        Self {
            len: source.len(),
            width: shape.width,
            text: source,
            content: Content::Projected(shape, hidden, ghost, indent),
        }
    }
    pub(super) fn hides_source(&self, index: usize) -> bool {
        matches!(&self.content,Content::Projected(_,hidden,_,_) if hidden.contains(&index))
    }
    pub(crate) fn projected_indent(&self) -> Option<Pixels> {
        if let Content::Projected(_, _, _, indent) = &self.content {
            Some(*indent)
        } else {
            None
        }
    }
    pub(crate) fn inline(text: SharedString, fragments: Vec<InlineFragment>) -> Self {
        let width = fragments.last().map_or(px(0.), |f| f.x + f.width);
        Self {
            len: text.len(),
            text,
            width,
            content: Content::Inline(fragments),
        }
    }
    pub(in crate::input) fn x_for_display_index(&self, ix: usize) -> Pixels {
        match &self.content {
            Content::Projected(line, ..) => line.x_for_index(ix),
            _ => self.x_for_index(ix),
        }
    }
    pub(in crate::input) fn x_for_source_start(&self, ix: usize) -> Pixels {
        if let Content::Projected(line, hidden, ghost, _) = &self.content
            && hidden.is_empty()
            && ix == hidden.start
        {
            return line.x_for_index(ghost.end);
        }
        self.x_for_index(ix)
    }
    pub(crate) fn x_for_index(&self, ix: usize) -> Pixels {
        match &self.content {
            Content::Projected(line, hidden, ghost, _) => {
                let display = if ix <= hidden.start {
                    ix
                } else if ix < hidden.end {
                    ghost.start
                } else {
                    ix - hidden.end + ghost.end
                };
                line.x_for_index(display)
            }
            Content::Text(line) => line.x_for_index(ix),
            Content::Inline(fragments) => {
                for f in fragments {
                    if ix < f.range.end {
                        return f.x
                            + f.text.as_ref().map_or(
                                if ix <= f.range.start { px(0.) } else { f.width },
                                |line| line.x_for_index(ix.saturating_sub(f.range.start)),
                            );
                    }
                }
                self.width
            }
        }
    }
    pub(super) fn closest_projected_column(&self, x: Pixels) -> Option<usize> {
        match &self.content {
            Content::Projected(line, ..) => Some(line.closest_index_for_x(x)),
            _ => None,
        }
    }
    pub(crate) fn closest_index_for_x(&self, x: Pixels) -> usize {
        match &self.content {
            Content::Projected(line, hidden, ghost, _) => {
                super::projection::source_boundary(line.closest_index_for_x(x), hidden, ghost)
            }
            Content::Text(line) => line.closest_index_for_x(x),
            Content::Inline(fragments) => {
                for f in fragments {
                    if x <= f.x + f.width {
                        return f.text.as_ref().map_or(
                            if x - f.x < f.width / 2. {
                                f.range.start
                            } else {
                                f.range.end
                            },
                            |line| f.range.start + line.closest_index_for_x(x - f.x),
                        );
                    }
                }
                self.len
            }
        }
    }
    pub(crate) fn index_for_x(&self, x: Pixels) -> Option<usize> {
        match &self.content {
            Content::Text(line) => line.index_for_x(x),
            Content::Projected(line, hidden, ghost, _) => line
                .index_for_x(x)
                .map(|i| super::projection::source_boundary(i, hidden, ghost)),
            Content::Inline(_) => {
                (x >= px(0.) && x <= self.width).then(|| self.closest_index_for_x(x))
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint_pass(pos, height, align, width, false, window, cx);
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_background(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint_pass(pos, height, align, width, true, window, cx);
    }
    #[allow(clippy::too_many_arguments)]
    fn paint_pass(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        background: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let paint = |line: &ShapedLine, pos, align, width, window: &mut Window, cx: &mut App| {
            if background {
                let _ = line.paint_background(pos, height, align, width, window, cx);
            } else {
                #[cfg(feature = "test-support")]
                crate::test_support::record_text(
                    line.text.clone(),
                    false,
                    gpui::Bounds::new(pos, gpui::size(line.width, height)),
                    window,
                );
                let _ = line.paint(pos, height, align, width, window, cx);
            }
        };
        match &self.content {
            Content::Projected(line, ..) | Content::Text(line) => {
                paint(line, pos, align, width, window, cx)
            }
            Content::Inline(fragments) => {
                let remaining = (width.unwrap_or(self.width) - self.width).max(px(0.));
                let offset = match align {
                    TextAlign::Right => remaining,
                    TextAlign::Center => remaining / 2.,
                    _ => px(0.),
                };
                for f in fragments {
                    if let Some(line) = &f.text {
                        paint(
                            line,
                            pos + point(offset + f.x, px(0.)),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            }
        }
    }
}
