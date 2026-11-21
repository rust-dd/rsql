// RSQL local source changes. See NOTICE for upstream provenance and modifications.
use crate::input::{EditorMode, InputBaseState, InputModeKind, RopeExt};
use gpui::{App, Hsla, SharedString};
use std::ops::Range;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorSurface {
    pub fill: Hsla,
    pub border: Hsla,
    pub foreground: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorPaint {
    pub inactive_selection: Hsla,
    pub line_number: Hsla,
    pub active_line_number: Hsla,
    pub guide: Hsla,
    pub active_guide: Hsla,
    pub widget: EditorSurface,
    pub hover: EditorSurface,
    pub input: EditorSurface,
}

impl<M: InputModeKind> InputBaseState<M> {
    pub fn editor_paint(&self) -> Option<EditorPaint> {
        self.paint_override
    }
}

impl InputBaseState<EditorMode> {
    /// Called when composing the editor. The next paint reads these overrides.
    pub fn set_editor_paint(&mut self, paint: Option<EditorPaint>) {
        self.paint_override = paint;
    }
}

/// A display-only replacement of a UTF-8 byte range in the source. Both the
/// range and replacement text may span logical lines. Endpoints must be
/// grapheme boundaries. An invalid range, masking, visible tokens or active
/// composition rejects the request and clears the previous projection.
#[derive(Clone, Debug, PartialEq)]
pub struct InlinePreview {
    pub range: Range<usize>,
    pub text: SharedString,
    pub color: Hsla,
}

impl InputBaseState<EditorMode> {
    /// Returns false and clears the current projection for an invalid source range,
    /// masked input, token ownership or active composition.
    pub fn set_inline_preview(&mut self, preview: Option<InlinePreview>, cx: &mut App) -> bool {
        let requested = preview.is_some();
        let boundary = |offset| {
            use crate::input::rope_ext::{next_grapheme_boundary, previous_grapheme_boundary};
            offset == 0
                || next_grapheme_boundary(
                    &self.text,
                    previous_grapheme_boundary(&self.text, offset),
                ) == offset
        };
        let preview = preview.filter(|p| {
            !self.tokens_visible()
                && self.ime_marked_range.is_none()
                && !self.masked
                && p.range.start <= p.range.end
                && p.range.end <= self.text.len()
                && self.text.clip_offset(p.range.start, sum_tree::Bias::Left) == p.range.start
                && self.text.clip_offset(p.range.end, sum_tree::Bias::Left) == p.range.end
                && boundary(p.range.start)
                && boundary(p.range.end)
        });
        let accepted = !requested || preview.is_some();
        let preview = preview.and_then(|p| {
            use unicode_segmentation::UnicodeSegmentation as _;
            let source = self.text.slice(p.range.clone()).to_string();
            let prefix = source
                .graphemes(true)
                .zip(p.text.graphemes(true))
                .take_while(|(a, b)| a == b)
                .map(|(g, _)| g.len())
                .sum::<usize>();
            let suffix = source[prefix..]
                .graphemes(true)
                .rev()
                .zip(p.text[prefix..].graphemes(true).rev())
                .take_while(|(a, b)| a == b)
                .map(|(g, _)| g.len())
                .sum::<usize>();
            let range = p.range.start + prefix..p.range.end - suffix;
            let text = &p.text[prefix..p.text.len() - suffix];
            (!range.is_empty() || !text.is_empty()).then(|| InlinePreview {
                range,
                text: text.to_owned().into(),
                color: p.color,
            })
        });
        if preview.is_some() {
            self.extras.inline_completion = Default::default();
        }
        if self.display_map.set_preview(preview, cx) {
            self.longest_line_width.set(None);
        }
        accepted
    }
}

#[cfg(feature = "test-support")]
impl InputBaseState<EditorMode> {
    pub fn first_line_whitespace(&self) -> Vec<(usize, gpui::Pixels, Option<gpui::Pixels>)> {
        self.last_layout.as_ref().unwrap().lines[0]
            .whitespace_chars
            .clone()
    }
    pub fn caret_affinity_and_color(&self) -> (bool, gpui::Hsla) {
        (self.cursor_line_end_affinity, self.editor_style.caret)
    }
}
