// RSQL local source changes. See NOTICE for upstream provenance and modifications.
use super::{
    InputLine, LineLayout,
    rows::{IndexedRuns, Rows},
};
use crate::input::{InlinePreview, tab_stops::expand_tabs};
use gpui::{Hsla, Pixels, TextRun, Window};
use smallvec::SmallVec;
use std::{ops::Range, rc::Rc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::input) enum Endpoint {
    Caret,
    RangeStart,
    RangeEnd,
}

pub(in crate::input) struct ProjectedLine {
    pub row: usize,
    pub end_row: usize,
    visual: String,
    paragraph_starts: Vec<usize>,
    rows: Vec<(Range<usize>, u32)>,
    hidden: Range<usize>,
    ghost: Range<usize>,
    color: Hsla,
}

impl ProjectedLine {
    pub(super) fn new(
        row: usize,
        end_row: usize,
        span: Range<usize>,
        source: &ropey::Rope,
        p: &InlinePreview,
    ) -> Self {
        #[cfg(feature = "test-support")]
        crate::input::observations::measure(|work| work.projections += 1);
        let hidden = p.range.start - span.start..p.range.end - span.start;
        let mut visual = String::with_capacity(span.len() - hidden.len() + p.text.len());
        for chunk in source.slice(span.start..p.range.start).chunks() {
            visual.push_str(chunk);
        }
        visual.push_str(&p.text);
        for chunk in source.slice(p.range.end..span.end).chunks() {
            visual.push_str(chunk);
        }
        let ghost = hidden.start..hidden.start + p.text.len();
        Self {
            row,
            end_row,
            paragraph_starts: std::iter::once(0)
                .chain(visual.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
            visual,
            rows: vec![],
            hidden,
            ghost,
            color: p.color,
        }
    }

    pub(super) fn wrap(
        &mut self,
        original: Option<&super::text_wrapper::LineItem>,
        mut measure: impl FnMut(&str) -> Vec<gpui::Boundary>,
    ) {
        // A stable ASCII word boundary in both layouts restarts the same native wrap.
        // Probe a bounded prefix with whole words beyond the join so the artificial
        // end of the probe cannot choose the join. Preserve the complete native
        // fallback for paragraph/indent changes and Unicode line-break contexts.
        if let Some(item) = original
            && item.wrapped_lines.len() > 64
            && self.row == self.end_row
            && self.ghost.end < 4096
            && !self.visual[..self.ghost.end].contains('\n')
            && self.visual.len() > 8192
            && self.visual.is_char_boundary(8192)
        {
            let prefix = &self.visual[..8192];
            if prefix.is_ascii()
                && !prefix.contains('\n')
                && let Some(end) = prefix.rfind(' ')
            {
                let indent = prefix
                    .bytes()
                    .take_while(|b| *b == b' ')
                    .count()
                    .min(gpui::LineWrapper::MAX_INDENT as usize);
                if indent == item.indent as usize && self.ghost.start >= indent {
                    let mut start = 0;
                    let mut hanging = 0;
                    for boundary in measure(&prefix[..end]) {
                        let cut = boundary.ix;
                        self.rows.push((start..cut, hanging));
                        start = cut;
                        hanging = boundary.next_indent;
                        if cut <= self.ghost.end
                            || cut + 2 >= end
                            || hanging != item.indent
                            || !prefix.as_bytes()[cut - 1].eq(&b' ')
                            || !prefix.as_bytes()[cut].is_ascii_alphanumeric()
                        {
                            continue;
                        }
                        let src = self.source_index(cut);
                        if let Ok(tail) = item.wrapped_lines.binary_search_by_key(&src, |r| r.start)
                        {
                            for row in &item.wrapped_lines[tail..] {
                                self.rows.push((
                                    row.start - self.hidden.end + self.ghost.end
                                        ..row.end - self.hidden.end + self.ghost.end,
                                    hanging,
                                ));
                            }
                            return;
                        }
                    }
                    self.rows.clear();
                }
            }
        }
        // An unindented ASCII paragraph can restart at a native word break.
        // Keep two complete words of lookahead past every emitted break. A short
        // bounded memo avoids remeasuring equal tiles in generated SQL literals.
        if self.visual.len() > 8192
            && self.visual.is_ascii()
            && !self.visual.contains('\n')
            && self
                .visual
                .as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_alphanumeric())
        {
            let mut memo = std::collections::HashMap::<&str, Vec<usize>>::new();
            let mut pos = 0;
            while self.visual.len() - pos > 4096 {
                let probe = &self.visual[pos..pos + 4096];
                let safe = probe
                    .as_bytes()
                    .windows(2)
                    .rposition(|v| v[0] == b' ' && v[1].is_ascii_alphanumeric());
                let Some(last_word) = safe else { break };
                let key = &probe[..last_word];
                let Some(last_break) = key.rfind(' ') else {
                    break;
                };
                if memo.len() >= 8 && !memo.contains_key(key) {
                    memo.clear();
                }
                let cuts = memo.entry(key).or_insert_with(|| {
                    measure(key)
                        .into_iter()
                        .map(|b| b.ix)
                        .take_while(|&ix| ix < last_break)
                        .collect()
                });
                let Some(last) = cuts.iter().rposition(|&i| {
                    i > 0
                        && key.as_bytes()[i - 1] == b' '
                        && key.as_bytes()[i].is_ascii_alphanumeric()
                }) else {
                    break;
                };
                let start = pos;
                for &cut in &cuts[..=last] {
                    self.rows.push((pos..start + cut, 0));
                    pos = start + cut;
                }
            }
            if pos > 0 {
                let start = pos;
                for boundary in measure(&self.visual[start..]) {
                    self.rows.push((pos..start + boundary.ix, 0));
                    pos = start + boundary.ix;
                }
                self.rows.push((pos..self.visual.len(), 0));
                return;
            }
        }
        let mut start = 0;
        for part in self.visual.split('\n') {
            let mut previous = 0;
            let mut indent = 0;
            for boundary in measure(part) {
                self.rows
                    .push((start + previous..start + boundary.ix, indent));
                previous = boundary.ix;
                indent = boundary.next_indent;
            }
            self.rows
                .push((start + previous..start + part.len(), indent));
            start += part.len() + 1;
        }
    }

    pub(super) fn fits_cache(&self, source_range_bytes: usize) -> bool {
        self.row == self.end_row
            && self.visual.capacity()
                + self.rows.capacity() * std::mem::size_of::<(Range<usize>, u32)>()
                + self.paragraph_starts.capacity() * std::mem::size_of::<usize>()
                + source_range_bytes
                <= 1_048_576
    }
    pub(super) fn source_ranges(&self) -> SmallVec<[Range<usize>; 1]> {
        let mut previous_end = 0;
        self.rows
            .iter()
            .map(|(row, _)| {
                let end = self.source_index(row.end);
                let start = std::mem::replace(&mut previous_end, end);
                start..end
            })
            .collect()
    }

    pub(super) fn visual_position(
        &self,
        source: usize,
        endpoint: Endpoint,
        line_end: bool,
    ) -> (usize, usize) {
        let visual = if source < self.hidden.start {
            source
        } else if source == self.hidden.start
            && !(self.hidden.is_empty() && endpoint == Endpoint::RangeStart)
        {
            self.ghost.start
        } else if source < self.hidden.end {
            self.ghost.start
        } else {
            source - self.hidden.end + self.ghost.end
        };
        let left = line_end || endpoint == Endpoint::RangeEnd;
        let mut index = self
            .rows
            .partition_point(|(r, _)| {
                if left {
                    r.end < visual
                } else {
                    r.end <= visual
                }
            })
            .min(self.rows.len().saturating_sub(1));
        if index > 0 && self.rows[index].0.start > visual {
            index -= 1;
        }
        let row = &self.rows[index].0;
        (index, visual.saturating_sub(row.start).min(row.len()))
    }

    pub(super) fn source_for_visual_position(&self, row: usize, col: usize) -> usize {
        let r = &self.rows[row].0;
        self.source_index(r.start + col.min(r.len()))
    }
    pub(super) fn source_index(&self, pos: usize) -> usize {
        source_boundary(pos, &self.hidden, &self.ghost)
    }

    pub(super) fn pointer_boundary(&self, row: usize, visual_col: usize) -> (usize, bool) {
        let range = &self.rows[row].0;
        let column = visual_col.min(range.len());
        let source = self.source_index(range.start + column);
        // Only an actual soft row end that paints this source caret upstream
        // can set upstream affinity. Interior virtual rows have no source caret.
        let upstream = column == range.len()
            && self
                .rows
                .get(row + 1)
                .is_some_and(|(next, _)| next.start == range.end)
            && self.visual_position(source, Endpoint::Caret, true) == (row, column);
        (source, upstream)
    }

    pub(in crate::input) fn shape(
        self: &Rc<Self>,
        source: ropey::Rope,
        base: usize,
        len: usize,
        runs: Vec<TextRun>,
        font_size: Pixels,
        tab_stop: Pixels,
        visible: Range<usize>,
        window: &Window,
    ) -> LineLayout {
        let bg = runs.iter().any(|run| run.background_color.is_some());
        let runs = IndexedRuns::new(runs);
        let mut ghost_run = window.text_style().to_run(self.ghost.len());
        ghost_run.color = self.color;
        let ranges = self.source_ranges().into_vec();
        let p = self.clone();
        let ts = window.text_system().clone();
        let build = move |i: usize| {
            #[cfg(feature = "test-support")]
            crate::input::observations::measure(|work| work.shaped_rows += 1);
            let (row, indent) = &p.rows[i];
            let start = if i == 0 {
                0
            } else {
                p.source_index(p.rows[i - 1].0.end)
            };
            let end = p.source_index(row.end);
            let within = |pos: usize| pos.clamp(row.start, row.end) - row.start;
            let visual_runs = |r: Range<usize>| {
                let left = r.start.min(p.ghost.start)..r.end.min(p.ghost.start);
                let right = r.start.max(p.ghost.end) - p.ghost.end + p.hidden.end
                    ..r.end.max(p.ghost.end) - p.ghost.end + p.hidden.end;
                let mut result = runs.slice(left);
                let ghost = r
                    .end
                    .min(p.ghost.end)
                    .saturating_sub(r.start.max(p.ghost.start));
                if ghost > 0 {
                    result.push(TextRun {
                        len: ghost,
                        ..ghost_run.clone()
                    });
                }
                if !right.is_empty() {
                    result.extend(runs.slice(right));
                }
                result
            };
            let text: gpui::SharedString = p.visual[row.clone()].to_owned().into();
            let styles = visual_runs(row.clone());
            let styles = crate::input::element::align_runs_to_char_boundaries(&text, &styles)
                .unwrap_or(styles);
            let mut shaped = ts.shape_line(text, font_size, &styles, None);
            expand_tabs(&mut shaped, tab_stop);
            let x = if *indent == 0 {
                gpui::px(0.)
            } else {
                let first = p.paragraph_starts[p
                    .paragraph_starts
                    .partition_point(|start| *start <= row.start)
                    - 1];
                let leading = first..first + *indent as usize;
                ts.shape_line(
                    p.visual[leading.clone()].to_owned().into(),
                    font_size,
                    &visual_runs(leading),
                    None,
                )
                .width
            };
            InputLine::projected(
                source.slice(base + start..base + end).to_string().into(),
                shaped,
                p.hidden.start.clamp(start, end) - start..p.hidden.end.clamp(start, end) - start,
                within(p.ghost.start)..within(p.ghost.end),
                x,
            )
        };
        LineLayout::new()
            .viewport_rows(Rows::viewport(ranges, visible, build), len)
            .with_projection(self.clone())
            .with_background(bg)
    }
}

pub(super) fn source_boundary(visual: usize, hidden: &Range<usize>, ghost: &Range<usize>) -> usize {
    if visual < ghost.start {
        visual
    } else if visual < ghost.end {
        hidden.start
    } else {
        visual - ghost.end + hidden.end
    }
}
