use super::super::{TerminalGeometry, style};
use crate::terminal::{Color, NamedColor, TerminalSnapshot};
use gpui_kit::{
    App, Bounds, Hsla, LineLayout, Pixels, Point, ShapedLine, SharedString, TextRun,
    UnderlineStyle, Window, component::ActiveTheme, font, point, px, size,
};
use std::ops::Range;

#[derive(Default)]
/// Owned by one immutable mark. A replacement constructs a fresh cache.
pub(super) struct LayoutCache(Option<MarkLayout>);

impl LayoutCache {
    pub fn get(
        &mut self,
        text: &SharedString,
        screen: &TerminalSnapshot,
        w: &Window,
        cx: &App,
    ) -> &MarkLayout {
        let theme = cx.theme();
        let ink = style::color(screen, Color::Named(NamedColor::Foreground));
        self.0.take_if(|layout| {
            layout.ink != ink
                || layout.family != theme.mono_font_family
                || layout.line.font_size != theme.mono_font_size
        });
        self.0.get_or_insert_with(|| {
            let family = theme.mono_font_family.clone();
            let run = TextRun {
                len: text.len(),
                font: font(family.clone()),
                color: ink,
                background_color: None,
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(ink),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = w
                .text_system()
                .shape_line(text.clone(), theme.mono_font_size, &[run], None);
            let positions = positions(&line, text.len());
            MarkLayout {
                line,
                positions,
                family,
                ink,
            }
        })
    }
}

pub(super) struct MarkLayout {
    pub line: ShapedLine,
    positions: Vec<Pixels>,
    family: SharedString,
    ink: Hsla,
}

fn positions(line: &LineLayout, display_len: usize) -> Vec<Pixels> {
    // A native shaping failure can return an empty layout for nonempty display text.
    let length = display_len + 1;
    let mut positions = Vec::with_capacity(length);
    // GPUI uses the first glyph with index >= the requested byte, in shaping order.
    // Each new maximum supplies the positions of the still-unfilled byte offsets up to it.
    for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
        let end = glyph.index.saturating_add(1).min(length);
        if end > positions.len() {
            positions.resize(end, glyph.position.x);
        }
    }
    positions.resize(length, line.width);
    positions
}

impl MarkLayout {
    fn x(&self, byte: usize) -> Pixels {
        self.positions[byte]
    }

    pub fn endpoints(&self, bytes: Range<usize>) -> (Pixels, Pixels) {
        (self.x(bytes.start), self.x(bytes.end))
    }

    pub fn origin(
        &self,
        active: usize,
        g: TerminalGeometry,
        caret: Bounds<Pixels>,
    ) -> Point<Pixels> {
        let right = g.content.right() - px(1.).min(g.content.size.width);
        point(caret.left().min(right - self.x(active)), caret.top())
    }

    pub fn bounds(
        &self,
        origin: Point<Pixels>,
        height: Pixels,
        range: Range<usize>,
        content: Bounds<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        if content.is_empty() {
            return None;
        }
        let (start, end) = self.endpoints(range);
        let start = origin.x + start;
        let end = origin.x + end;
        let width = px(1.).min(content.size.width);
        let left = start
            .min(end)
            .clamp(content.left(), content.right() - width);
        let right = start.max(end).max(left + width).min(content.right());
        let bounds =
            Bounds::new(point(left, origin.y), size(right - left, height)).intersect(&content);
        (!bounds.is_empty()).then_some(bounds)
    }
}
