use std::{ops::Range, panic::Location};

use gpui::{
    App, ElementId, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled, StyledText,
    Window, div, prelude::FluentBuilder, rems,
};

use gpui_base::{LayoutDirection, LogicalStyledExt as _};
use gpui_base::{TestSupportExt as _, TestTextExt as _};

use crate::{ActiveTheme, StyledExt};

const MASKED: &'static str = "•";

/// Represents the type of match for highlighting text in a label.
#[derive(Clone)]
pub enum HighlightsMatch {
    Prefix(SharedString),
    Full(SharedString),
}

impl HighlightsMatch {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Prefix(s) => s.as_str(),
            Self::Full(s) => s.as_str(),
        }
    }

    #[inline]
    pub fn is_prefix(&self) -> bool {
        matches!(self, Self::Prefix(_))
    }
}

impl From<&str> for HighlightsMatch {
    fn from(value: &str) -> Self {
        Self::Full(value.to_string().into())
    }
}

impl From<String> for HighlightsMatch {
    fn from(value: String) -> Self {
        Self::Full(value.into())
    }
}

impl From<SharedString> for HighlightsMatch {
    fn from(value: SharedString) -> Self {
        Self::Full(value)
    }
}

/// A text label element with optional secondary text, masking, and highlighting capabilities.
///
/// A label is exposed to assistive technology as a `Label` node whose value is
/// its text, so a screen reader can read it. A masked or empty label has no
/// node.
#[derive(IntoElement)]
pub struct Label {
    id: ElementId,
    style: StyleRefinement,
    label: SharedString,
    secondary: Option<SharedString>,
    masked: bool,
    highlights_text: Option<HighlightsMatch>,
}

impl Label {
    /// Create a new label with the main label.
    ///
    /// The label's id defaults to this call site, which is unique wherever one
    /// line builds one label. See [`Label::id`] for labels built in a loop.
    #[track_caller]
    pub fn new(label: impl Into<SharedString>) -> Self {
        let label: SharedString = label.into();
        Self {
            id: ElementId::CodeLocation(*Location::caller()),
            style: Default::default(),
            label,
            secondary: None,
            masked: false,
            highlights_text: None,
        }
    }

    /// Set the label's element id, replacing the default taken from where
    /// [`Label::new`] was called.
    ///
    /// Pass one where a single `Label::new` renders several labels as
    /// siblings, such as in a loop: they share the default id, so assistive
    /// technology gets only the first of them, and GPUI's debug builds panic
    /// on the duplicate while it is connected. The id must be unique among
    /// those siblings. Labels under parents that have their own ids, like the
    /// rows of a `List`, are already distinct.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the secondary text for the label,
    /// the secondary text will be displayed after the label text with `muted` color.
    pub fn secondary(mut self, secondary: impl Into<SharedString>) -> Self {
        self.secondary = Some(secondary.into());
        self
    }

    /// Set whether to mask the label text.
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    /// Set for matching text to highlight in the label.
    pub fn highlights(mut self, text: impl Into<HighlightsMatch>) -> Self {
        self.highlights_text = Some(text.into());
        self
    }

    fn full_text(&self) -> SharedString {
        match &self.secondary {
            Some(secondary) => format!("{} {}", self.label, secondary).into(),
            None => self.label.clone(),
        }
    }

    fn highlight_ranges(&self, total_length: usize) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let full_text = self.full_text();

        if self.secondary.is_some() {
            ranges.push(0..self.label.len());
            ranges.push(self.label.len()..total_length);
        }

        if let Some(matched) = &self.highlights_text {
            let matched_str = matched.as_str();
            if !matched_str.is_empty() {
                let search_lower = matched_str.to_lowercase();
                let full_text_lower = full_text.to_lowercase();

                if matched.is_prefix() {
                    // For prefix matching, only check if the text starts with the search term
                    if full_text_lower.starts_with(&search_lower) {
                        ranges.push(0..matched_str.len());
                    }
                } else {
                    // For full matching, find all occurrences
                    let mut search_start = 0;
                    while let Some(pos) = full_text_lower[search_start..].find(&search_lower) {
                        let match_start = search_start + pos;
                        let match_end = match_start + matched_str.len();

                        if match_end <= full_text.len() {
                            ranges.push(match_start..match_end);
                        }

                        search_start = match_start + 1;
                        while !full_text.is_char_boundary(search_start)
                            && search_start < full_text.len()
                        {
                            search_start += 1;
                        }

                        if search_start >= full_text.len() {
                            break;
                        }
                    }
                }
            }
        }

        ranges
    }

    fn measure_highlights(
        &self,
        length: usize,
        cx: &mut App,
    ) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
        if self.masked {
            return None;
        }

        let ranges = self.highlight_ranges(length);
        if ranges.is_empty() {
            return None;
        }

        let mut highlights = Vec::new();
        let mut highlight_ranges_added = 0;

        if self.secondary.is_some() {
            highlights.push((ranges[0].clone(), HighlightStyle::default()));
            highlights.push((
                ranges[1].clone(),
                HighlightStyle {
                    color: Some(cx.theme().muted_foreground),
                    ..Default::default()
                },
            ));
            highlight_ranges_added = 2;
        }

        for range in ranges.iter().skip(highlight_ranges_added) {
            highlights.push((
                range.clone(),
                HighlightStyle {
                    color: Some(cx.theme().blue),
                    ..Default::default()
                },
            ));
        }

        Some(gpui::combine_highlights(vec![], highlights).collect())
    }
}

impl Styled for Label {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Label {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut text = self.full_text();
        let chars_count = text.chars().count();

        if self.masked {
            text = SharedString::from(MASKED.repeat(chars_count))
        };

        let highlights = self.measure_highlights(text.len(), cx);
        // Plain text is not in the accessibility tree, so the label carries
        // it. A masked label keeps its text out of the tree.
        let accessible_text = (!self.masked && !text.is_empty()).then(|| text.clone());

        div()
            .id(self.id)
            .test_support()
            .when_some(accessible_text, |this, text| {
                // A `Label` node is named by its value, not its label.
                this.role(gpui::Role::Label).aria_value(text)
            })
            .line_height(rems(1.25))
            .text_color(cx.theme().foreground)
            // Start-aligned in RTL; LTR keeps the inherited alignment.
            .when(LayoutDirection::global(cx).is_rtl(), |this| {
                this.text_start(cx)
            })
            .refine_style(&self.style)
            .child(
                StyledText::new(&text).when_some(highlights, |this, hl| this.with_highlights(hl)),
            )
            .test_text(text)
    }
}

/// `text` with every occurrence of the search `query` drawn in the theme's
/// `list_highlight_foreground` color, matched the way searchable lists filter:
/// by case-insensitive substring.
pub(crate) fn highlight_matches(text: SharedString, query: &str, cx: &App) -> StyledText {
    let style = HighlightStyle {
        color: Some(cx.theme().list_highlight_foreground),
        ..Default::default()
    };
    let highlights = match_ranges(&text, query)
        .into_iter()
        .map(|range| (range, style))
        .collect::<Vec<_>>();

    StyledText::new(text).with_highlights(highlights)
}

/// The byte ranges of `text` that case-insensitively equal `query`, without
/// overlaps.
fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let mut ranges = Vec::new();
    if query.is_empty() {
        return ranges;
    }

    let mut next_start = 0;
    for (start, _) in text.char_indices() {
        if start < next_start {
            continue;
        }
        if let Some(len) = match_len(&text[start..], &query) {
            ranges.push(start..start + len);
            next_start = start + len;
        }
    }

    ranges
}

/// The byte length of the start of `text` whose lowercase form is `query`.
fn match_len(text: &str, query: &[char]) -> Option<usize> {
    let mut rest = query;
    for (offset, ch) in text.char_indices() {
        for lower in ch.to_lowercase() {
            let (first, tail) = rest.split_first()?;
            if *first != lower {
                return None;
            }
            rest = tail;
        }
        if rest.is_empty() {
            return Some(offset + ch.len_utf8());
        }
    }

    None
}
