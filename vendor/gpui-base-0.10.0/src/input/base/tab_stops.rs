//! Tab stops for shaped lines.
//!
//! The platform shapers do not agree on what a `\t` is: DirectWrite gives it
//! no width at all, CoreText uses its own default stops. Neither knows the
//! editor's tab size. So a line is shaped as is and its tabs are then widened
//! to the next tab stop, by moving every glyph after a tab to the right. The
//! caret, hit testing and selection all read glyph positions, so they follow.

use std::sync::Arc;

use gpui::{Font, LineLayout, Pixels, ShapedGlyph, ShapedLine, ShapedRun, WindowTextSystem, px};

/// The distance between two tab stops: `tab_size` spaces of `font`.
pub(crate) fn tab_stop_width(
    text_system: &WindowTextSystem,
    font: &Font,
    font_size: Pixels,
    tab_size: usize,
) -> Pixels {
    let font_id = text_system.resolve_font(font);
    text_system.layout_width(font_id, font_size, ' ') * tab_size as f32
}

/// Widens each tab of `line` to end on the next multiple of `stop`.
///
/// Stops are counted from the start of `line`, so a soft-wrapped row starts
/// its own stops, the same way the wrapper measures it.
pub(crate) fn expand_tabs(line: &mut ShapedLine, stop: Pixels) {
    if let Some(layout) = expanded_layout(line, &line.text, stop) {
        **line = Arc::new(layout);
    }
}

/// The width of `layout` (shaped from `text`) once its tabs are expanded.
pub(crate) fn expanded_width(layout: &LineLayout, text: &str, stop: Pixels) -> Pixels {
    if stop <= px(0.) {
        return layout.width;
    }

    layout.width
        + tab_shifts(layout, text, stop)
            .last()
            .map_or(px(0.), |(_, shift)| shift)
}

/// Cumulative shifts after each tab, in text order.
fn tab_shifts<'a>(
    layout: &'a LineLayout,
    text: &'a str,
    stop: Pixels,
) -> impl Iterator<Item = (usize, Pixels)> + 'a {
    let mut glyphs = layout.runs.iter().flat_map(|run| &run.glyphs).peekable();
    // Tab boundaries increase monotonically. Keep the cursor between lookups
    // instead of restarting LineLayout::x_for_index's scan for every tab.
    let mut x_for_index = move |index| {
        while glyphs.peek().is_some_and(|glyph| glyph.index < index) {
            glyphs.next();
        }
        glyphs.peek().map_or(layout.width, |glyph| glyph.position.x)
    };

    // Keep the cumulative shift in double precision. Rounding it to Pixels
    // after every tab can put a stop just below its boundary, making the next
    // tab land on that same stop instead of advancing.
    let stop = f64::from(f32::from(stop));
    text.match_indices('\t').scan(0_f64, move |shift, (ix, _)| {
        let start = f64::from(f32::from(x_for_index(ix))) + *shift;
        let end = f64::from(f32::from(x_for_index(ix + 1)));
        let next_stop = stop * ((start / stop).floor() + 1.);
        *shift = next_stop - end;
        Some((ix, px(*shift as f32)))
    })
}

/// `layout` with its tabs widened, or `None` when it has none to widen.
fn expanded_layout(layout: &LineLayout, text: &str, stop: Pixels) -> Option<LineLayout> {
    if stop <= px(0.) || !text.contains('\t') {
        return None;
    }

    let shifts: Vec<_> = tab_shifts(layout, text, stop).collect();
    let shift = shifts.last().map_or(px(0.), |(_, shift)| *shift);

    let shift_after = |index: usize| {
        let tabs_before = shifts.partition_point(|&(tab, _)| tab < index);
        tabs_before
            .checked_sub(1)
            .map_or(px(0.), |last| shifts[last].1)
    };

    let runs = layout
        .runs
        .iter()
        .map(|run| ShapedRun {
            font_id: run.font_id,
            glyphs: run
                .glyphs
                .iter()
                .map(|glyph| ShapedGlyph {
                    position: gpui::point(
                        glyph.position.x + shift_after(glyph.index),
                        glyph.position.y,
                    ),
                    ..glyph.clone()
                })
                .collect(),
        })
        .collect();

    Some(LineLayout {
        font_size: layout.font_size,
        width: layout.width + shift,
        ascent: layout.ascent,
        descent: layout.descent,
        runs,
        len: layout.len,
    })
}
