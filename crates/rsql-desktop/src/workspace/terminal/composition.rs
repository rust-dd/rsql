use super::{TerminalGeometry, TerminalView, style};
use crate::terminal::{
    Color, NamedColor, TerminalSnapshot,
    session::{CommandError, Edit, MAX_INPUT_BYTES},
};
use gpui_kit::*;
use std::{borrow::Cow, ops::Range};

#[path = "composition_index.rs"]
mod index;
use index::Utf16Index;

#[path = "composition_layout.rs"]
mod layout;
use layout::LayoutCache;

#[derive(Default)]
pub(super) struct Composition {
    text: SharedString,
    display: SharedString,
    layout: LayoutCache,
    index: Utf16Index,
    selection: Range<usize>,
    last_ime_anchor: Option<Bounds<Pixels>>,
}

impl Composition {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn replacement(
        &self,
        range: Option<Range<usize>>,
        text: &str,
    ) -> Result<(String, usize), CommandError> {
        let target = self.index.range(range.unwrap_or(0..usize::MAX));
        let len = self.text.len() - target.bytes.len();
        if text.len() > MAX_INPUT_BYTES - len {
            return Err(CommandError::TooLarge);
        }
        let mut result = String::with_capacity(len + text.len());
        result.push_str(&self.text[..target.bytes.start]);
        result.push_str(text);
        result.push_str(&self.text[target.bytes.end..]);
        Ok((result, target.units.start))
    }

    pub fn paint(
        &mut self,
        g: TerminalGeometry,
        screen: &TerminalSnapshot,
        selection: Hsla,
        w: &mut Window,
        cx: &mut App,
    ) {
        if self.is_empty() {
            return;
        }
        let Some(caret) = g.caret_cell(screen) else {
            self.last_ime_anchor = None;
            return;
        };
        let layout = self.layout.get(&self.display, screen, w, cx);
        let range = self.index.range(self.selection.clone()).bytes;
        let origin = layout.origin(range.end, g, caret);
        // GPUI positions its candidate panel at the start of the marked visual line.
        let anchor = layout.bounds(origin, caret.size.height, 0..0, g.content);
        if self.last_ime_anchor != anchor {
            self.last_ime_anchor = anchor;
            if anchor.is_some() {
                w.invalidate_character_coordinates();
            }
        }
        w.with_content_mask(Some(ContentMask { bounds: g.content }), |w| {
            let mut bounds = Bounds::new(origin, caret.size);
            bounds.size.width = layout.line.width.max(px(1.));
            w.paint_quad(fill(bounds, style::background(screen)));
            let (start, end) = layout.endpoints(range.clone());
            bounds.origin.x += start.min(end);
            bounds.size.width = (end - start).abs().max(px(1.));
            w.paint_quad(fill(
                bounds,
                if range.is_empty() {
                    style::color(screen, Color::Named(NamedColor::Cursor))
                } else {
                    selection
                },
            ));
            let _ = layout
                .line
                .paint(origin, caret.size.height, TextAlign::Left, None, w, cx);
        });
    }
}

impl EntityInputHandler for TerminalView {
    fn accepts_text_input(&self, w: &mut Window, _: &mut Context<Self>) -> bool {
        self.can_type(w)
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        w: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if !self.can_type(w) {
            return None;
        }
        let mark = &self.input.composition;
        let range = mark.index.range(range);
        *adjusted = Some(range.units);
        Some(mark.text[range.bytes].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        w: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        self.can_type(w).then(|| UTF16Selection {
            range: self.input.composition.selection.clone(),
            reversed: false,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.can_type(w) {
            self.input.composition.selection = self.input.composition.index.range(range).units;
            cx.notify();
        }
    }

    fn text_length_utf16(&mut self, w: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.can_type(w).then(|| self.input.composition.index.len())
    }

    fn marked_text_range(&self, w: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (self.can_type(w) && !self.input.composition.is_empty())
            .then(|| 0..self.input.composition.index.len())
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_type(w) {
            return;
        }
        match self.input.composition.replacement(range, text) {
            Ok((result, offset)) => {
                if result.is_empty() {
                    self.unmark_text(w, cx);
                    return;
                }
                if !self.apply_edit(Edit::Preedit, cx) {
                    return;
                }
                let index = Utf16Index::new(&result);
                let end = index.unit(self.input.composition.index.byte(offset) + text.len());
                let selected = selection.unwrap_or(usize::MAX..usize::MAX);
                let start = offset.saturating_add(selected.start).min(end);
                let end = offset.saturating_add(selected.end).min(end);
                let selection = index.range(start..end).units;
                let text = SharedString::from(result);
                let display = if text.contains(['\r', '\n']) {
                    text.replace(['\r', '\n'], " ").into()
                } else {
                    text.clone()
                };
                self.input.composition = Composition {
                    text,
                    display,
                    layout: LayoutCache::default(),
                    index,
                    selection,
                    last_ime_anchor: self.input.composition.last_ime_anchor,
                };
                cx.notify();
            }
            Err(e) => {
                self.admission(Err(e), cx);
            }
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_type(w) {
            return;
        }
        let commit = if self.input.composition.is_empty() {
            Cow::Borrowed(text)
        } else {
            match self.input.composition.replacement(range, text) {
                Ok((result, _)) => Cow::Owned(result),
                Err(error) => {
                    self.admission(Err(error), cx);
                    return;
                }
            }
        };
        if commit.is_empty() || self.apply_edit(Edit::Text(commit.as_bytes()), cx) {
            self.input.composition = Default::default();
            cx.notify();
        }
    }

    fn unmark_text(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.can_type(w) {
            self.input.composition = Default::default();
            cx.notify();
        }
    }

    fn paste(&mut self, item: ClipboardItem, w: &mut Window, cx: &mut Context<Self>) {
        if self.can_type(w) && self.input.composition.is_empty() {
            self.paste_item(item, cx);
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.ime_bounds(range, w, cx)
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if !self.can_type(w) {
            return None;
        }
        let screen = self.frame.screen.as_deref()?;
        let g = self.geometry()?;
        let caret = g.caret_cell(screen)?;
        if !g.content.contains(&position)
            || position.y < caret.top()
            || position.y >= caret.bottom()
        {
            return None;
        }
        let mark = &mut self.input.composition;
        let layout = mark.layout.get(&mark.display, screen, w, cx);
        let origin = layout.origin(mark.index.byte(mark.selection.end), g, caret);
        let index = layout.line.closest_index_for_x(position.x - origin.x);
        Some(mark.index.unit(index))
    }
}

impl TerminalView {
    fn ime_bounds(&mut self, range: Range<usize>, w: &Window, cx: &App) -> Option<Bounds<Pixels>> {
        if !self.can_type(w) {
            return None;
        }
        let g = self.geometry()?;
        let screen = self.frame.screen.as_deref()?;
        let caret = g.caret_cell(screen)?;
        let mark = &mut self.input.composition;
        if mark.is_empty() {
            let bounds = caret.intersect(&g.content);
            return (!bounds.is_empty()).then_some(bounds);
        }
        let layout = mark.layout.get(&mark.display, screen, w, cx);
        let origin = layout.origin(mark.index.byte(mark.selection.end), g, caret);
        layout.bounds(
            origin,
            caret.size.height,
            mark.index.range(range).bytes,
            g.content,
        )
    }
}
