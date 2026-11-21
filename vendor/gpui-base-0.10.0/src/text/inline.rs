mod selection_paint;

use super::state::ParagraphRows;
#[cfg(target_family = "wasm")]
use async_channel::bounded;
use gpui::Corners;
#[cfg(not(target_family = "wasm"))]
use smol::channel::bounded;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem,
    ops::Range,
    rc::Rc,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui::{
    AnyTooltip, App, BorderStyle, Bounds, ClickEvent, CursorStyle, Edges, Element, ElementId,
    ForegroundExecutor, Global, GlobalElementId, Half, HighlightStyle, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, LayoutId, MouseButton, MouseClickEvent, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyleRefinement, StyledText, Task,
    TextAlign, TextLayout, TextRun, TextStyle, Window, point, px, size,
};

use crate::{
    GlobalState, TextSelection,
    async_util::Sender,
    input::Selection,
    text::TextViewMultiClickKind,
    text::node::LinkMark,
    text::range_highlight::RevealAt,
    text::selection::word_range_at,
    text::state::LineSpan,
    text::text_view::{LinkClickHandlerFn, handle_link_click},
    text_selection::text_rows_extent,
};

/// The style applied to one range of inline text.
///
/// A [`HighlightStyle`] carries no font family, so the family an inline code
/// span is set in rides beside it; `None` keeps the family of the enclosing
/// text style.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct InlineHighlight {
    pub(super) style: HighlightStyle,
    pub(super) font_family: Option<SharedString>,
    pub(super) font_size_scale: Option<f32>,
    /// Typography and box refinements for an inline code span.
    pub(super) code_style: Option<StyleRefinement>,
}

impl InlineHighlight {
    pub(super) fn font_size(&self, body: Pixels, rem_size: Pixels) -> Pixels {
        self.code_style
            .as_ref()
            .and_then(|style| style.text.font_size)
            .map(|size| size.to_pixels(rem_size))
            .filter(|size| size.as_f32().is_finite() && *size > Pixels::ZERO)
            .unwrap_or(body * self.font_size_scale.unwrap_or(1.))
    }

    /// Insets used by shaping, wrapping, painting, and table measurement alike.
    pub(super) fn code_insets(&self, font_size: Pixels, rem_size: Pixels) -> Edges<Pixels> {
        if self.font_size_scale.is_none() {
            return Edges::default();
        }
        let mut insets = Edges {
            top: Pixels::ZERO,
            bottom: Pixels::ZERO,
            left: px(super::inline_flow::INLINE_CODE_PADDING),
            right: px(super::inline_flow::INLINE_CODE_PADDING),
        };
        if let Some(style) = &self.code_style {
            for (inset, padding, border) in [
                (&mut insets.top, style.padding.top, style.border_widths.top),
                (
                    &mut insets.right,
                    style.padding.right,
                    style.border_widths.right,
                ),
                (
                    &mut insets.bottom,
                    style.padding.bottom,
                    style.border_widths.bottom,
                ),
                (
                    &mut insets.left,
                    style.padding.left,
                    style.border_widths.left,
                ),
            ] {
                if let Some(padding) = padding {
                    *inset = padding
                        .to_pixels(font_size.into(), rem_size)
                        .max(Pixels::ZERO);
                }
                if let Some(border) = border {
                    *inset += border.to_pixels(rem_size).max(Pixels::ZERO);
                }
            }
        }
        insets
    }

    /// Layers `other` over `self`, the way [`HighlightStyle::highlight`] does.
    fn highlight(mut self, other: &InlineHighlight) -> Self {
        self.style = self.style.highlight(other.style);
        if other.font_family.is_some() {
            self.font_family = other.font_family.clone();
        }
        if other.font_size_scale.is_some() {
            self.font_size_scale = other.font_size_scale;
        }
        if other.code_style.is_some() {
            self.code_style = other.code_style.clone();
        }
        self
    }
}

impl From<HighlightStyle> for InlineHighlight {
    fn from(style: HighlightStyle) -> Self {
        Self {
            style,
            font_family: None,
            font_size_scale: None,
            code_style: None,
        }
    }
}

/// Merges two highlight lists over one text into non-overlapping ranges,
/// cutting at every endpoint of every input range. Same sweep as
/// [`gpui::combine_highlights`], for [`InlineHighlight`] payloads.
pub(super) fn combine_highlights(
    a: impl IntoIterator<Item = (Range<usize>, InlineHighlight)>,
    b: impl IntoIterator<Item = (Range<usize>, InlineHighlight)>,
) -> Vec<(Range<usize>, InlineHighlight)> {
    let mut endpoints = Vec::new();
    let mut highlights = Vec::new();
    for (range, highlight) in a.into_iter().chain(b) {
        if !range.is_empty() {
            let id = highlights.len();
            endpoints.push((range.start, id, true));
            endpoints.push((range.end, id, false));
            highlights.push(highlight);
        }
    }
    endpoints.sort_unstable_by_key(|(position, _, _)| *position);

    let mut combined = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut ix = 0;
    for (position, id, is_start) in endpoints {
        if position > ix && !active.is_empty() {
            let style = active.iter().fold(InlineHighlight::default(), |acc, id| {
                acc.highlight(&highlights[*id])
            });
            combined.push((ix..position, style));
        }
        ix = position;
        if is_start {
            active.push(id);
        } else {
            active.retain(|active_id| *active_id != id);
        }
    }
    combined
}

/// Layers host foregrounds after mark/link styling, independent of where a
/// mark starts. `combine_highlights` folds active spans in start order, so a
/// mark beginning inside an already active foreground must not win its color.
pub(super) fn foreground_highlights(
    highlights: Vec<(Range<usize>, InlineHighlight)>,
    text: &str,
    offset: usize,
    foregrounds: &[(Range<usize>, Hsla)],
) -> Vec<(Range<usize>, InlineHighlight)> {
    let ranges: Vec<_> = foregrounds
        .iter()
        .filter_map(|(range, color)| {
            let start = range.start.saturating_sub(offset).min(text.len());
            let end = range.end.saturating_sub(offset).min(text.len());
            (start < end && text.is_char_boundary(start) && text.is_char_boundary(end))
                .then_some((start..end, *color))
        })
        .collect();
    if ranges.is_empty() {
        return highlights;
    }
    let mut highlights = combine_highlights(
        highlights,
        ranges.iter().map(|(range, color)| {
            (
                range.clone(),
                HighlightStyle {
                    color: Some(*color),
                    ..Default::default()
                }
                .into(),
            )
        }),
    );
    for (range, highlight) in &mut highlights {
        if let Some((_, color)) = ranges.iter().rev().find(|(r, _)| r.contains(&range.start)) {
            highlight.style.color = Some(*color);
        }
    }
    highlights
}

/// Layers a streamed fade-in over `highlights`: text inside each fade range
/// loses that share of its color, and a highlight background or decoration
/// color fades with it so an inline code chip or a link underline does not
/// appear before its text.
pub(super) fn fade_highlights(
    highlights: Vec<(Range<usize>, InlineHighlight)>,
    fades: &[(Range<usize>, f32)],
) -> Vec<(Range<usize>, InlineHighlight)> {
    if fades.is_empty() {
        return highlights;
    }
    let fade_highlights = fades.iter().map(|(range, fade_out)| {
        (
            range.clone(),
            InlineHighlight::from(HighlightStyle {
                fade_out: Some(*fade_out),
                ..Default::default()
            }),
        )
    });
    let mut combined = combine_highlights(highlights, fade_highlights);
    // GPUI fades only the glyph color, so explicit decoration colors fade
    // here too, or a link underline would show before its text does.
    for (_, highlight) in &mut combined {
        let Some(fade_out) = highlight.style.fade_out else {
            continue;
        };
        if let Some(code) = &mut highlight.code_style {
            if let Some(background) = code.background.as_ref().and_then(gpui::Fill::color) {
                code.background = Some(background.opacity(1. - fade_out).into());
            }
            if let Some(border) = &mut code.border_color {
                border.fade_out(fade_out);
            }
        }
        let style = &mut highlight.style;
        for color in [
            style.background_color.as_mut(),
            style.underline.as_mut().and_then(|u| u.color.as_mut()),
            style.strikethrough.as_mut().and_then(|s| s.color.as_mut()),
        ]
        .into_iter()
        .flatten()
        {
            color.fade_out(fade_out);
        }
    }
    combined
}

/// Builds the [`TextRun`]s for `text_len` bytes of inline text: each
/// highlight refines `default_style` over its range, and a highlight that
/// names a font family shapes its run in that family.
pub(super) fn text_runs(
    text_len: usize,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, InlineHighlight)],
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0;
    for (range, highlight) in highlights {
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        let mut style = default_style.clone().highlight(highlight.style);
        if let Some(code) = &highlight.code_style {
            // Font size is handled by the shaped fragment, not by a TextRun.
            style.font_features = code
                .text
                .font_features
                .clone()
                .unwrap_or(style.font_features);
            style.font_fallbacks = code.text.font_fallbacks.clone().or(style.font_fallbacks);
        }
        let mut run = style.to_run(range.len());
        if let Some(family) = &highlight.font_family {
            run.font.family = family.clone();
        }
        runs.push(run);
        ix = range.end;
    }
    if ix < text_len {
        runs.push(default_style.to_run(text_len - ix));
    }
    runs
}

pub(super) fn code_insets(
    highlights: &[(Range<usize>, InlineHighlight)],
    font_size: Pixels,
    rem_size: Pixels,
) -> Edges<Pixels> {
    highlights
        .iter()
        .find(|(_, highlight)| highlight.font_size_scale.is_some())
        .map(|(_, highlight)| highlight.code_insets(font_size, rem_size))
        .unwrap_or_default()
}

/// Splits text into contiguous ranges sharing one font size. GPUI runs can
/// vary the font but not its size, so each range needs its own shaped line.
pub(super) fn text_size_ranges(
    text_len: usize,
    highlights: &[(Range<usize>, InlineHighlight)],
    font_size: Pixels,
    rem_size: Pixels,
) -> Vec<(Range<usize>, f32)> {
    let mut ranges: Vec<(Range<usize>, f32, bool)> = Vec::new();
    let mut push = |range: Range<usize>, scale: f32, code: bool| {
        if range.is_empty() {
            return;
        }
        if let Some((last, last_scale, last_code)) = ranges.last_mut()
            && *last_scale == scale
            && *last_code == code
            && last.end == range.start
        {
            last.end = range.end;
        } else {
            ranges.push((range, scale, code));
        }
    };
    let mut cursor = 0;
    for (range, highlight) in highlights {
        push(cursor..range.start, 1., false);
        push(
            range.clone(),
            highlight.font_size(font_size, rem_size) / font_size,
            highlight.font_size_scale.is_some(),
        );
        cursor = range.end;
    }
    push(cursor..text_len, 1., false);
    ranges
        .into_iter()
        .map(|(range, scale, _)| (range, scale))
        .collect()
}

/// A inline element used to render a inline text and support selectable.
///
/// All text in TextView (including the CodeBlock) used this for text rendering.
pub(super) struct Inline {
    text: SharedString,
    links: Rc<Vec<(Range<usize>, LinkMark)>>,
    highlights: Vec<(Range<usize>, InlineHighlight)>,
    styled_text: StyledText,
    /// The resolved style from a parent deferred layout, when there is one.
    text_style: Option<TextStyle>,
    paint_origin: Option<Point<Pixels>>,
    selection_bounds: Option<Bounds<Pixels>>,
    /// What a triple click selects, when the text is one row of a wrapped
    /// inline flow: the whole paragraph, as its first and last rows.
    paragraph_rows: Option<ParagraphRows>,
    selection_source: Option<(Arc<Mutex<InlineState>>, Range<usize>)>,
    /// Range highlight backgrounds, painted behind the text.
    range_backgrounds: Vec<(Range<usize>, Hsla)>,
    /// The start of a pending reveal, when it is in this text.
    reveal: Option<RevealAt>,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    link_tooltip: bool,
    /// What this frame's layout was shaped with, to hand the shaped text to
    /// the next frame (see [`RetainedLayout`]).
    retained_key: Option<(Vec<TextRun>, TextStyle)>,
    /// The shaped text is in the table, not in `styled_text`, until paint
    /// takes it back.
    handed_over: bool,
    /// A fragment of an editor leaf laid out by `InlineFlow`: the leaf's key
    /// and the leaf byte offset of this text's index 0.
    editor_piece: Option<(u64, usize)>,

    state: Arc<Mutex<InlineState>>,
}

/// The inline text state, used RefCell to keep the selection state.
#[derive(Debug, Default)]
pub(crate) struct InlineState {
    /// The text that actually rendering, matched with selection.
    pub(super) text: SharedString,
    pub(super) selection: Option<Selection>,
    pub(super) selection_rows: Option<Box<selection_paint::Rows>>,
    /// Set only through the editor host API (`Paragraph::with_leaf_key`):
    /// the leaf reports its geometry to the collector installed by
    /// `editor_host::collect_leaf_geometry`. `None` for every plain `TextView`.
    pub(crate) editor_key: Option<u64>,
    /// UI owners of retained layouts; cleanup metadata is not document state.
    pub(super) cleanup_senders: Vec<Sender<()>>,
}

impl PartialEq for InlineState {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.selection == other.selection
            && self.editor_key == other.editor_key
    }
}

/// One frame's [`StyledText`], kept for the next frame's [`Inline`] of the
/// same [`InlineState`].
///
/// A [`TextLayout`] remembers the size and the shaped lines of its last
/// measurement and answers a repeated measure at the same wrap width from
/// them, but a fresh `StyledText` every frame throws that away, so every
/// frame of a scroll paid for a line wrapper, a shaping-cache lookup that
/// hashes the whole paragraph, and the allocations around them for every
/// visible paragraph. Handing the same `StyledText` to the next frame makes
/// those measurements hits. A layout is only reused when the text, the runs
/// (colors, fades, fonts) and the text style (font size, line height) it was
/// shaped with are unchanged; a different wrap width misses inside
/// `TextLayout` and reshapes as before.
///
/// `StyledText` is main-thread only (an `Rc` inside), while `InlineState`
/// travels through the background parse, so the layouts live in a
/// thread-local table keyed by the state's address, with a `Weak` to tell a
/// live state from a reused address.
struct RetainedLayout {
    state: Weak<Mutex<InlineState>>,
    styled_text: StyledText,
    text: SharedString,
    runs: Vec<TextRun>,
    text_style: TextStyle,
}

thread_local! {
    static RETAINED_LAYOUTS: RefCell<HashMap<usize, RetainedLayout>> = RefCell::new(HashMap::new());
    /// [`DROPPED_STATES`] when this thread's table was last swept.
    static SWEPT_AT_DROPS: Cell<usize> = const { Cell::new(0) };
}

/// Dead entries (states that were dropped without a final paint, e.g. a
/// replaced document) are swept once the table grows past this many, or by
/// the UI cleanup task after an [`InlineState`] was dropped.
const RETAINED_SWEEP_AT: usize = 4096;

/// How many [`InlineState`]s have been dropped. States drop on whichever
/// thread lets go of them last (the background parse included), so this is a
/// global count; each thread's table remembers the count it last swept at
/// ([`SWEPT_AT_DROPS`]), so that one thread's sweep does not use up another's.
static DROPPED_STATES: AtomicUsize = AtomicUsize::new(0);

/// Owns the App's retained-layout notifications and foreground sweep task.
struct RetainedLayoutCleanup {
    /// Closed before teardown so late state drops cannot wake the task.
    sender: Sender<()>,
    /// Cancelled while its executor is still alive.
    _task: Task<()>,
    // App drops its executors before globals. Closing the channel can wake
    // the task, so keep its scheduler alive until after channel/task teardown;
    // otherwise the wake drops the receiver under the channel's event lock.
    _executor: ForegroundExecutor,
}

impl Global for RetainedLayoutCleanup {}

impl Drop for RetainedLayoutCleanup {
    fn drop(&mut self) {
        // Release the local task's channel waker on its owning thread before
        // cancelling it. Another app's drops must not wake a retired executor.
        self.sender.close();
    }
}

pub(super) fn init(cx: &mut App) {
    if cx.has_global::<RetainedLayoutCleanup>() {
        return;
    }
    sweep_retained_layouts();

    // One pending notification is enough to sweep every dead entry. In
    // particular, closing a document must not require another Inline to draw.
    let (sender, receiver) = bounded(1);
    let task = cx.foreground_executor().spawn(async move {
        while receiver.recv().await.is_ok() {
            sweep_retained_layouts();
        }
    });
    cx.set_global(RetainedLayoutCleanup {
        sender,
        _task: task,
        _executor: cx.foreground_executor().clone(),
    });
}

fn state_key(state: &Arc<Mutex<InlineState>>) -> usize {
    Arc::as_ptr(state) as usize
}

/// Takes the layout retained for `state`, if the previous frame left one.
fn take_retained_layout(state: &Arc<Mutex<InlineState>>) -> Option<RetainedLayout> {
    RETAINED_LAYOUTS.with(|layouts| {
        let retained = layouts.borrow_mut().remove(&state_key(state))?;
        // The address may belong to a new state by now.
        retained
            .state
            .upgrade()
            .is_some_and(|live| Arc::ptr_eq(&live, state))
            .then_some(retained)
    })
}

/// Whether the table already holds a layout for `state`: every entry left
/// by the previous frame is taken at layout time, so one that is present
/// afterwards was put there this frame, by another element of the same state.
fn has_retained_layout(state: &Arc<Mutex<InlineState>>) -> bool {
    RETAINED_LAYOUTS.with(|layouts| layouts.borrow().contains_key(&state_key(state)))
}

fn sweep_retained_layouts() {
    RETAINED_LAYOUTS.with(|layouts| {
        let mut layouts = layouts.borrow_mut();
        // Read before sweeping: a state dropped during the sweep wakes another
        // cleanup and must not be acknowledged by this pass.
        let dropped = DROPPED_STATES.load(Ordering::Acquire);
        if dropped != SWEPT_AT_DROPS.get() || layouts.len() >= RETAINED_SWEEP_AT {
            layouts.retain(|_, retained| retained.state.strong_count() > 0);
            SWEPT_AT_DROPS.set(dropped);
        }
    });
}

fn retain_layout(
    state: &Arc<Mutex<InlineState>>,
    retained: RetainedLayout,
    cleanup: Option<&Sender<()>>,
) {
    if let Some(cleanup) = cleanup
        && let Ok(mut state) = state.lock()
    {
        state.cleanup_senders.retain(|sender| !sender.is_closed());
        if !state
            .cleanup_senders
            .iter()
            .any(|sender| sender.same_channel(cleanup))
        {
            state.cleanup_senders.push(cleanup.clone());
        }
    }
    sweep_retained_layouts();
    RETAINED_LAYOUTS.with(|layouts| {
        layouts.borrow_mut().insert(state_key(state), retained);
    });
}

impl Drop for InlineState {
    fn drop(&mut self) {
        // A state can drop on the parser thread after its last frame. Wake
        // the UI owners even when there will be no further layout hand-over.
        DROPPED_STATES.fetch_add(1, Ordering::Release);
        for sender in &self.cleanup_senders {
            _ = sender.try_send(());
        }
    }
}

impl InlineState {
    /// Save actually rendered text for selected text to use.
    pub(crate) fn set_text(&mut self, text: SharedString) {
        self.text = text;
    }
}

/// The native lines and placement of a completed paint. `TextLayout::clone`
/// shares mutable measurement state; these immutable shaped lines do not.
struct PaintedSelectionLayout {
    lines: smallvec::SmallVec<[Arc<gpui::WrappedLineLayout>; 1]>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    len: usize,
}

impl PaintedSelectionLayout {
    fn new(layout: &TextLayout) -> Self {
        Self {
            lines: layout.line_layouts(),
            bounds: layout.bounds(),
            line_height: layout.line_height(),
            len: layout.len(),
        }
    }

    fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        let mut origin = self.bounds.origin;
        let mut start = 0;
        for line in &self.lines {
            let end = start + line.len();
            if index < start {
                break;
            }
            if index <= end {
                return Some(origin + line.position_for_index(index - start, self.line_height)?);
            }
            origin.y += line.size(self.line_height).height;
            start = end + 1;
        }
        None
    }

    fn rows_extent(&self) -> (Pixels, Pixels) {
        let top = self.bounds.top();
        let mut last_line_top = top;
        for line in self.lines.iter().take(self.lines.len().saturating_sub(1)) {
            last_line_top += line.size(self.line_height).height;
        }
        let last_row_top = self.lines.last().map_or(top, |line| {
            last_line_top + line.wrap_boundaries.len() as f32 * self.line_height
        });
        (top, last_row_top + self.line_height)
    }
}

/// Targets the document-owned source state, never an ephemeral flow fragment.
pub(super) struct InlineSelectionProjection {
    source: Weak<Mutex<InlineState>>,
    range: Range<usize>,
    text: SharedString,
    layout: PaintedSelectionLayout,
    selection_bounds: Option<Bounds<Pixels>>,
}

impl InlineSelectionProjection {
    pub(super) fn reset(&self) {
        if let Some(source) = self.source.upgrade()
            && let Ok(mut source) = source.lock()
        {
            source.selection = None;
        }
    }

    pub(super) fn project(&self, start: Point<Pixels>, end: Point<Pixels>) {
        let Some(source) = self.source.upgrade() else {
            return;
        };
        let selection =
            project_selection(&self.text, &self.layout, self.selection_bounds, start, end);
        if let Some(selection) = selection
            && let Ok(mut source) = source.lock()
        {
            let start = self.range.start + selection.start;
            let end = self.range.start + selection.end;
            source.selection = Some(match source.selection {
                Some(previous) => Selection::new(previous.start.min(start), previous.end.max(end)),
                None => Selection::new(start, end),
            });
        }
    }
}

impl Inline {
    /// Editor seam (design D3): report this leaf's layout and take the
    /// editor's backgrounds. Inert unless the state carries an editor key
    /// and an editor sink is installed.
    fn report_to_editor(&mut self) {
        // The sink (a thread-local read) is checked first: outside an
        // editor's prepaint no lock is taken.
        let Some(sink) = super::editor_seam::current() else {
            return;
        };
        if let Some((key, start)) = self.editor_piece {
            let layout = self.styled_text.layout();
            let end = start + layout.len();
            sink.report_text_piece(key, start, layout);
            self.range_backgrounds
                .extend(
                    sink.backgrounds(key)
                        .into_iter()
                        .filter_map(|(range, color)| {
                            let (lo, hi) = (range.start.max(start), range.end.min(end));
                            (lo < hi).then(|| (lo - start..hi - start, color))
                        }),
                );
            return;
        }
        let Some(key) = self.state.lock().ok().and_then(|state| state.editor_key) else {
            return;
        };
        sink.report(key, self.styled_text.layout());
        self.range_backgrounds.extend(sink.backgrounds(key));
    }

    /// Hands the shaped text to the next frame (see [`RetainedLayout`]).
    /// Called after prepaint, so an element that is laid out but never
    /// painted (scrolled out of view) keeps its layout too; paint takes it
    /// back for the duration of painting.
    ///
    /// When another element of the same state already handed one over this
    /// frame (the same document shown twice), this one keeps its own: the
    /// table holds one layout per state, and an element must never be left
    /// to paint without its shaped text.
    fn retain_styled_text(&mut self, cx: &App) {
        if self.handed_over || self.retained_key.is_none() || has_retained_layout(&self.state) {
            return;
        }
        let Some((runs, text_style)) = self.retained_key.take() else {
            return;
        };
        retain_layout(
            &self.state,
            RetainedLayout {
                state: Arc::downgrade(&self.state),
                styled_text: mem::replace(&mut self.styled_text, StyledText::new("")),
                text: self.text.clone(),
                runs,
                text_style,
            },
            cx.try_global::<RetainedLayoutCleanup>()
                .map(|cleanup| &cleanup.sender),
        );
        self.handed_over = true;
    }

    /// Takes the shaped text back from the table for painting. `false` when
    /// it is gone, in which case there is nothing to paint with.
    fn reclaim_styled_text(&mut self) -> bool {
        if !self.handed_over {
            return true;
        }
        let Some(retained) = take_retained_layout(&self.state) else {
            return false;
        };
        self.styled_text = retained.styled_text;
        self.retained_key = Some((retained.runs, retained.text_style));
        self.handed_over = false;
        true
    }

    pub(super) fn new(
        state: Arc<Mutex<InlineState>>,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, InlineHighlight)>,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    ) -> Self {
        let text = state
            .lock()
            .map(|state| state.text.clone())
            .unwrap_or_default();

        Self {
            links: Rc::new(links),
            highlights,
            text: text.clone(),
            styled_text: StyledText::new(text),
            text_style: None,
            paint_origin: None,
            selection_bounds: None,
            paragraph_rows: None,
            selection_source: None,
            range_backgrounds: Vec::new(),
            reveal: None,
            link_click_handler,
            link_tooltip: false,
            retained_key: None,
            handed_over: false,
            editor_piece: None,
            state,
        }
    }

    /// Use the resolved style captured by a deferred parent layout.
    pub(super) fn text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = Some(text_style);
        self
    }

    /// Report this text as a fragment of the editor leaf `key`, starting at
    /// the leaf's byte `start` (see `report_to_editor`).
    pub(super) fn editor_piece(mut self, piece: Option<(u64, usize)>) -> Self {
        self.editor_piece = piece;
        self
    }

    /// Preserve the shared inline-flow baseline through GPUI's element-bound snapping.
    pub(super) fn paint_origin(mut self, origin: Point<Pixels>) -> Self {
        self.paint_origin = Some(origin);
        self
    }

    pub(super) fn selection_bounds(mut self, bounds: Bounds<Pixels>) -> Self {
        self.selection_bounds = Some(bounds);
        self
    }

    pub(super) fn paragraph_rows(mut self, rows: ParagraphRows) -> Self {
        self.paragraph_rows = Some(rows);
        self
    }

    pub(super) fn selection_source(
        mut self,
        state: Arc<Mutex<InlineState>>,
        range: Range<usize>,
    ) -> Self {
        self.selection_source = Some((state, range));
        self
    }

    /// Paint `backgrounds` behind the text. They are not part of the text
    /// runs, so changing them does not shape the text again.
    pub(super) fn range_backgrounds(mut self, backgrounds: Vec<(Range<usize>, Hsla)>) -> Self {
        self.range_backgrounds = backgrounds;
        self
    }

    /// Scroll the line `reveal` starts on into view during prepaint.
    pub(super) fn reveal(mut self, reveal: Option<RevealAt>) -> Self {
        self.reveal = reveal;
        self
    }

    /// Ask the enclosing list to scroll the line of the pending reveal into
    /// view, and report where it is and whether it is inside the visible
    /// area.
    fn request_reveal(&self, window: &mut Window) {
        let Some(reveal) = &self.reveal else {
            return;
        };
        let text_layout = self.styled_text.layout();
        let bounds = text_layout.bounds();
        let line_height = text_layout.line_height();
        let glyphs = glyph_boxes(
            text_layout,
            window.text_style().text_align,
            bounds.size.width,
        );
        // The glyph drawing the text at the offset, or, for text with no
        // glyph of its own such as a line break, the next glyph, or the last.
        let offset = reveal.offset();
        let (row, left, right) = range_boxes(&glyphs, offset..offset + 1)
            .first()
            .copied()
            .or_else(|| {
                glyphs
                    .iter()
                    .find(|glyph| glyph.text.start >= offset)
                    .or(glyphs.last())
                    .map(|glyph| (glyph.row, glyph.left, glyph.right))
            })
            .unwrap_or((0, Pixels::ZERO, Pixels::ZERO));
        let line = Bounds::from_corners(
            point(
                bounds.left() + left,
                bounds.top() + line_height * row as f32,
            ),
            point(
                bounds.left() + right.max(left + px(1.)),
                bounds.top() + line_height * (row + 1) as f32,
            ),
        );
        window.request_autoscroll(line);
        // A list scrolls the line to its edge, which layout may miss by a
        // fraction of a pixel.
        let visible = window.content_mask().bounds.dilate(px(0.5));
        reveal.report(
            line,
            line.top() >= visible.top() && line.bottom() <= visible.bottom(),
        );
    }

    /// Get link at given mouse position.
    fn link_for_position(
        layout: &TextLayout,
        links: &Vec<(Range<usize>, LinkMark)>,
        position: Point<Pixels>,
    ) -> Option<LinkMark> {
        let offset = layout.index_for_position(position).ok()?;
        for (range, link) in links.iter() {
            if range.contains(&offset) {
                return Some(link.clone());
            }
        }

        None
    }

    fn prepaint_link_tooltip(&mut self, hitbox: &Hitbox, window: &mut Window, cx: &mut App) {
        let Some(view) = GlobalState::global(cx).text_view_state() else {
            return;
        };
        let Some(builder) = view.read(cx).link_tooltip.clone() else {
            return;
        };
        self.link_tooltip = !self.links.is_empty();
        // Hit testing is unavailable during prepaint, as with native GPUI tooltips.
        let position = window.mouse_position();
        if !self.link_tooltip
            || window.last_input_was_keyboard()
            || !hitbox.bounds.contains(&position)
            || !hitbox.content_mask.bounds.contains(&position)
        {
            return;
        }
        let Some(link) = Self::link_for_position(self.styled_text.layout(), &self.links, position)
        else {
            return;
        };
        let target = ((state_key(&self.state), hitbox.bounds), link.url.clone());
        if view.read(cx).link_tooltip_target.as_ref() != Some(&target) {
            return;
        }
        let cached = view
            .read(cx)
            .hovered_link_tooltip
            .as_ref()
            .filter(|(url, _)| *url == link.url)
            .map(|(_, tooltip)| tooltip.clone());
        let tooltip = match cached {
            Some(tooltip) => tooltip,
            None => {
                let tooltip = builder(&link.url, window, cx);
                view.update(cx, |state, _| {
                    state.hovered_link_tooltip = Some((link.url.clone(), tooltip.clone()));
                });
                tooltip
            }
        };
        let bounds = hitbox.bounds.intersect(&hitbox.content_mask.bounds);
        let layout = self.styled_text.layout().clone();
        let links = self.links.clone();
        window.set_tooltip(AnyTooltip {
            view: tooltip,
            mouse_position: position,
            check_visible_and_update: Rc::new(move |_, window, _| {
                let position = window.mouse_position();
                !window.last_input_was_keyboard()
                    && bounds.contains(&position)
                    && Self::link_for_position(&layout, &links, position).is_some()
            }),
        });
    }

    /// Get the range of the link at given mouse position.
    fn link_range_for_position(
        layout: &TextLayout,
        links: &[(Range<usize>, LinkMark)],
        position: Point<Pixels>,
    ) -> Option<Range<usize>> {
        let offset = layout.index_for_position(position).ok()?;
        links
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .map(|(range, _)| range.clone())
    }

    /// Paint selected bounds for debug.
    #[allow(unused)]
    fn paint_selected_bounds(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        window.paint_quad(gpui::PaintQuad {
            bounds,
            background: gpui::hsla(0.58, 0.85, 0.62, 0.01).into(),
            corner_radii: Corners::default(),
            border_color: gpui::transparent_black(),
            border_style: BorderStyle::default(),
            border_widths: gpui::Edges::all(px(0.)),
        });
    }

    fn layout_selections(
        &self,
        text_layout: &TextLayout,
        bounds: &Bounds<Pixels>,
        cx: &mut App,
    ) -> (bool, bool, Option<Selection>) {
        let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
            return (false, false, None);
        };

        let text_view_state = text_view_state.read(cx);
        let is_selectable = text_view_state.is_selectable();
        if !is_selectable {
            return (false, false, None);
        }

        if text_view_state.is_all_selected() {
            return (is_selectable, true, Some((0..self.text.len()).into()));
        }

        if text_view_state.preserve_inline_selection {
            let selection = if let Some((source, range)) = &self.selection_source {
                source
                    .lock()
                    .ok()
                    .and_then(|state| state.selection)
                    .and_then(|selection| {
                        let start = selection.start.max(range.start);
                        let end = selection.end.min(range.end);
                        (start < end)
                            .then(|| Selection::new(start - range.start, end - range.start))
                    })
            } else {
                self.state.lock().ok().and_then(|state| state.selection)
            };
            return (true, selection.is_some(), selection);
        }

        if let Some(selection) = text_view_state.multi_click_selection() {
            if selection.kind == TextViewMultiClickKind::Line {
                return (
                    true,
                    true,
                    selection
                        .line_bounds
                        .filter(|row| row.contains(&bounds.center()))
                        .map(|_| Selection::new(0, self.text.len())),
                );
            }
            return (
                is_selectable,
                true,
                selection_for_multi_click(
                    &self.text,
                    text_layout,
                    *bounds,
                    selection.pos,
                    selection.kind,
                )
                .map(Selection::from),
            );
        }

        let Some((selection_start, selection_end)) = text_view_state.painted_selection_points(cx)
        else {
            return (is_selectable, false, None);
        };
        (
            true,
            true,
            project_selection(
                &self.text,
                text_layout,
                self.selection_bounds,
                selection_start,
                selection_end,
            ),
        )
    }

    /// One box per laid-out row, from the row's start to its last character,
    /// clipped to `mask_bounds`.
    ///
    /// Walks the wrapped line layouts rather than every character:
    /// [`TextLayout::position_for_index`] scans a line's rows and glyphs on
    /// each call, so a per-character walk cost O(chars × glyphs) for every
    /// selectable inline on every frame — the largest single cost of painting
    /// a long chat message while it scrolls.
    ///
    /// A row followed by another row (a wrap or a newline) is extended by half
    /// a line height past its last glyph: the character walk gave that width
    /// to a character whose successor sits on the next row, and the selection
    /// geometry was tuned against it.
    fn text_line_bounds(
        text_layout: &PaintedSelectionLayout,
        line_height: Pixels,
        mask_bounds: Bounds<Pixels>,
    ) -> Vec<Bounds<Pixels>> {
        let origin = text_layout.bounds.origin;
        let lines = &text_layout.lines;
        let row_count: usize = lines
            .iter()
            .map(|line| line.wrap_boundaries.len() + 1)
            .sum();
        let mut line_bounds = Vec::with_capacity(row_count);
        let mut row_ix = 0;
        let mut y = origin.y;
        for line in lines {
            let layout = &line.unwrapped_layout;
            let mut row_start = 0;
            let row_ends = line
                .wrap_boundaries
                .iter()
                .map(|boundary| layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index)
                .chain([line.len()]);
            for row_end in row_ends {
                let mut width = layout.x_for_index(row_end) - layout.x_for_index(row_start);
                row_ix += 1;
                if row_ix < row_count {
                    width += line_height.half();
                }
                let bounds = Bounds::new(point(origin.x, y), size(width, line_height))
                    .intersect(&mask_bounds);
                if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                    line_bounds.push(bounds);
                }
                y += line_height;
                row_start = row_end;
            }
        }
        line_bounds
    }

    /// The caret line boxes at the two ends of a painted selection, where the
    /// touch handles are drawn.
    fn selection_edges(
        selection: &Selection,
        text_layout: &TextLayout,
    ) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        let (start, end) = (
            selection.start.min(selection.end),
            selection.start.max(selection.end),
        );
        let line_height = text_layout.line_height();
        Some((
            crate::touch_selection::caret_line_box(
                text_layout.position_for_index(start)?,
                line_height,
            ),
            crate::touch_selection::caret_line_box(
                text_layout.position_for_index(end)?,
                line_height,
            ),
        ))
    }

    /// Paint each range highlight behind the text of its range.
    fn paint_range_highlights(&self, text_layout: &TextLayout, window: &mut Window) {
        let glyphs = glyph_boxes(
            text_layout,
            window.text_style().text_align,
            text_layout.bounds().size.width,
        );
        let origin = text_layout.bounds().origin;
        let line_height = text_layout.line_height();
        for (range, color) in &self.range_backgrounds {
            for (row, left, right) in range_boxes(&glyphs, range.clone()) {
                window.paint_quad(gpui::fill(
                    Bounds::from_corners(
                        point(origin.x + left, origin.y + line_height * row as f32),
                        point(origin.x + right, origin.y + line_height * (row + 1) as f32),
                    ),
                    *color,
                ));
            }
        }
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl gpui::IntoElement for Inline {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Inline {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_element_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = self
            .text_style
            .clone()
            .unwrap_or_else(|| window.text_style());
        let runs = text_runs(self.text.len(), &text_style, &self.highlights);

        // Reuse the previous frame's shaped text when it was shaped from the
        // same text, runs and style; `StyledText` consumes its runs on every
        // layout, so they are handed over again either way.
        let retained = take_retained_layout(&self.state).filter(|retained| {
            retained.text == self.text && retained.runs == runs && retained.text_style == text_style
        });
        self.styled_text = match retained {
            Some(retained) => retained.styled_text.with_runs(runs.clone()),
            None => StyledText::new(self.text.clone()).with_runs(runs.clone()),
        };
        self.retained_key = Some((runs, text_style));
        let (layout_id, _) =
            self.styled_text
                .request_layout(global_element_id, inspector_id, window, cx);

        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = Bounds::new(self.paint_origin.unwrap_or(bounds.origin), bounds.size);
        self.styled_text
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);
        self.report_to_editor();

        // Report this element's laid-out extent so an ancestor TextView with
        // `max_lines` can snap its clip to a whole-line boundary. The state
        // stack only holds an entry during prepaint when that view set
        // `max_lines`, so this is a no-op otherwise.
        if let Some(text_view_state) = GlobalState::global(cx).text_view_state() {
            let state = text_view_state.read(cx);
            if state.max_lines.is_some()
                && let Ok(mut line_spans) = state.line_spans.lock()
            {
                line_spans.push(LineSpan {
                    top: bounds.top(),
                    bottom: bounds.bottom(),
                    line_height: window.line_height(),
                });
            }
        }

        self.request_reveal(window);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.prepaint_link_tooltip(&hitbox, window, cx);
        self.retain_styled_text(cx);
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let bounds = Bounds::new(self.paint_origin.unwrap_or(bounds.origin), bounds.size);
        #[cfg(feature = "test-support")]
        crate::test_support::record_text(self.text.clone(), false, bounds, window);
        let current_view = window.current_view();
        let hitbox = prepaint;
        if !self.reclaim_styled_text() {
            // Cannot happen (only this element takes what it handed over,
            // and a live state is never swept); skip the frame rather than
            // paint an unmeasured placeholder.
            return;
        }
        let text_layout = self.styled_text.layout().clone();
        if !self.range_backgrounds.is_empty() {
            self.paint_range_highlights(&text_layout, window);
        }
        _ = text_layout.paint_background(window, cx);

        // layout selections
        let (is_selectable, is_selection, selection) =
            self.layout_selections(&text_layout, &bounds, cx);

        let Ok(mut state) = self.state.lock() else {
            _ = text_layout.paint_foreground(window, cx);
            return;
        };

        state.selection = selection;
        if let Some((source, range)) = &self.selection_source
            && let Some(selection) = selection
            && let Ok(mut source) = source.lock()
        {
            let start = range.start + selection.start;
            let end = range.start + selection.end;
            source.selection = Some(match source.selection {
                Some(previous) => Selection::new(previous.start.min(start), previous.end.max(end)),
                None => Selection::new(start, end),
            });
        }

        let shows_text_cursor = GlobalState::global(cx)
            .text_view_state()
            .is_some_and(|view| view.read(cx).shows_text_cursor());
        if (is_selection || is_selectable) && shows_text_cursor {
            window.set_cursor_style(CursorStyle::IBeam, &hitbox);
        }

        // link cursor pointer. Only text under the pointer reads where the pointer
        // is: under GPUI's retained views, reading it makes the text depend on
        // every pointer move, so a transcript of markdown would be drawn again
        // whenever the pointer moved anywhere.
        let hovered_link = hitbox
            .is_hovered(window)
            .then(|| {
                Self::link_range_for_position(&text_layout, &self.links, window.mouse_position())
            })
            .flatten();
        if hovered_link.is_some() {
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
        }

        if self.link_tooltip
            && let Some(view) = GlobalState::global(cx).text_view_state()
        {
            let key = (state_key(&self.state), hitbox.bounds);
            let target = hovered_link.as_ref().and_then(|range| {
                self.links
                    .iter()
                    .find(|(candidate, _)| candidate == range)
                    .map(|(_, link)| (key, link.url.clone()))
            });
            let changed = view.update(cx, |state, _| {
                let owns_target = state
                    .link_tooltip_target
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner == key);
                if (target.is_some() || owns_target) && state.link_tooltip_target != target {
                    state.link_tooltip_target = target;
                    true
                } else {
                    false
                }
            });
            if changed {
                cx.notify(current_view);
                window.request_animation_frame();
            }
        }

        if let Some(selection) = state.selection {
            let color = GlobalState::global(cx)
                .text_view_state()
                .map(|state| state.read(cx).text_view_style.selection())
                .unwrap_or_else(|| crate::Theme::global(cx).tokens.colors.selection);
            selection_paint::visit(
                &text_layout,
                window.text_style().text_align,
                selection.start.min(selection.end)..selection.start.max(selection.end),
                window.content_mask().bounds,
                &mut state.selection_rows,
                |rect| window.paint_quad(gpui::fill(rect, color)),
            );
            if let Some((start, end)) = Self::selection_edges(&selection, &text_layout)
                && let Some(text_view_state) = GlobalState::global(cx).text_view_state()
            {
                text_view_state
                    .read(cx)
                    .selection_adapter
                    .register_selection_edges(start, end);
            }
        } else {
            state.selection_rows = None;
        }

        _ = text_layout.paint_foreground(window, cx);

        if is_selectable {
            if let Some(text_view_state) = GlobalState::global(cx).text_view_state() {
                let copy_layout = PaintedSelectionLayout::new(&text_layout);
                let text_bounds = Self::text_line_bounds(
                    &copy_layout,
                    text_layout.line_height(),
                    window.content_mask().bounds,
                );
                let adapter = &text_view_state.read(cx).selection_adapter;
                adapter.register_inline(text_bounds);
                adapter.register_text_run(
                    crate::TextSelectionRun::new(
                        self.text.clone(),
                        text_layout.clone(),
                        hitbox.bounds,
                    )
                    .with_paragraph(self.selection_bounds),
                );
                let (source, range) = self
                    .selection_source
                    .as_ref()
                    .map(|(source, range)| (source, range.clone()))
                    .unwrap_or((&self.state, 0..self.text.len()));
                adapter.register_copy_projection(
                    super::selection_adapter::CopyProjection::Text(InlineSelectionProjection {
                        source: Arc::downgrade(source),
                        range,
                        text: self.text.clone(),
                        layout: copy_layout,
                        selection_bounds: self.selection_bounds,
                    }),
                    cx,
                );
            }

            window.on_mouse_event({
                let hitbox = hitbox.clone();
                let text_layout = text_layout.clone();
                let inline_state = self.state.clone();
                let text = self.text.clone();
                let text_view_state = GlobalState::global(cx).text_view_state();
                let line_bounds = self.selection_bounds;
                let paragraph_rows = self
                    .paragraph_rows
                    .or_else(|| line_bounds.map(ParagraphRows::single));
                move |event: &MouseDownEvent, phase, window, cx| {
                    if !phase.bubble()
                        || !hitbox.is_hovered(window)
                        || event.button != MouseButton::Left
                    {
                        return;
                    }

                    if event.click_count == 3
                        && let Some(rows) = paragraph_rows
                    {
                        GlobalState::suppress_text_selection(cx);
                        if let Some(view) = &text_view_state {
                            view.update(cx, |state, cx| state.set_multi_click_line(rows, cx));
                        }
                        cx.notify(current_view);
                        return;
                    }

                    // A finger selects read-only text with a long press only;
                    // a double tap selects nothing here, neither the mouse's
                    // plain word nor the window layer's touch selection. The
                    // handles and the menu on a double tap belong to `Input`.
                    if event.click_count == 2 && GlobalState::is_touch_press(cx) {
                        GlobalState::suppress_text_selection(cx);
                        return;
                    }

                    let kind = match event.click_count {
                        2 => TextViewMultiClickKind::Word,
                        3 => TextViewMultiClickKind::Paragraph,
                        _ => return,
                    };

                    let Some(range) = selection_for_multi_click(
                        &text,
                        &text_layout,
                        hitbox.bounds,
                        event.position,
                        kind,
                    ) else {
                        return;
                    };
                    let Some(carets) = range_carets(&text_layout, &range, line_bounds) else {
                        return;
                    };

                    let selected_text = text[range.clone()].to_string();

                    // This renderer owns multi-click selection. Prevent the
                    // window selection layer from handling the same press.
                    GlobalState::suppress_text_selection(cx);

                    if let Ok(mut inline_state) = inline_state.lock() {
                        inline_state.selection = Some(range.into());
                    }
                    if let Some(text_view_state) = &text_view_state {
                        text_view_state.update(cx, |state, cx| {
                            state.set_multi_click_selection(
                                event.position,
                                kind,
                                selected_text,
                                carets,
                                cx,
                            );
                        });
                    }
                    cx.notify(current_view);
                }
            });
        }

        // Mouse move: repaint only when the pointer enters, leaves or moves
        // between links, so the link cursor follows it. Hovering plain text
        // changes nothing painted.
        if !self.links.is_empty() {
            window.on_mouse_event({
                let hitbox = hitbox.clone();
                let text_layout = text_layout.clone();
                let links = self.links.clone();
                let link_tooltip = self.link_tooltip;
                let mut hovered_link = hovered_link;
                move |event: &MouseMoveEvent, phase, window, cx| {
                    if !phase.bubble() {
                        return;
                    }
                    if !hitbox.is_hovered(window) {
                        if link_tooltip && hovered_link.take().is_some() {
                            cx.notify(current_view);
                        }
                        return;
                    }

                    let updated =
                        Self::link_range_for_position(&text_layout, &links, event.position);
                    if hovered_link != updated || (link_tooltip && updated.is_some()) {
                        hovered_link = updated;
                        cx.notify(current_view);
                    }
                }
            });
        }

        if !is_selection {
            // click to open link
            window.on_mouse_event({
                let links = self.links.clone();
                let text_layout = text_layout.clone();
                let hitbox = hitbox.clone();
                let text_view_state = GlobalState::global(cx).text_view_state();
                let link_click_handler = self.link_click_handler.clone();

                move |event: &MouseUpEvent, phase, window, cx| {
                    if !phase.bubble() || !hitbox.is_hovered(window) {
                        return;
                    }
                    if text_view_state
                        .as_ref()
                        .is_some_and(|state| state.read(cx).has_selection(cx))
                    {
                        return;
                    }

                    if let Some(link) =
                        Self::link_for_position(&text_layout, &links, event.position)
                    {
                        TextSelection::end(window, cx);
                        cx.stop_propagation();
                        let click = ClickEvent::Mouse(MouseClickEvent {
                            down: MouseDownEvent {
                                button: event.button,
                                position: event.position,
                                modifiers: event.modifiers,
                                click_count: event.click_count,
                                first_mouse: false,
                            },
                            up: event.clone(),
                        });
                        handle_link_click(&link_click_handler, link.url, click, window, cx);
                    }
                }
            });
        }

        drop(state);
        self.retain_styled_text(cx);
    }
}

/// Where one glyph of laid-out text paints: its row, and its horizontal
/// extent past the text's origin, alignment applied. `text` is the byte range
/// of the text it draws.
#[derive(Clone, Debug, PartialEq)]
struct GlyphBox {
    text: Range<usize>,
    row: usize,
    left: Pixels,
    right: Pixels,
}

/// The glyphs of `text_layout`, sorted by the text they draw, each placed
/// the way GPUI paints it: every row aligned in `align_width` by `align`,
/// and a glyph reaching to the next one on its row, or to the row's end.
///
/// Glyphs are read in the order they paint, so right-to-left text, whose
/// glyphs paint in the reverse order of its text, is placed as it shows.
fn glyph_boxes(text_layout: &TextLayout, align: TextAlign, align_width: Pixels) -> Vec<GlyphBox> {
    let mut boxes = Vec::new();
    let mut row = 0;
    let mut line_start = 0;
    for line in text_layout.line_layouts() {
        let layout = &line.unwrapped_layout;
        let glyphs = layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .collect::<Vec<_>>();
        // Each row starts at a wrap boundary glyph, and ends where the next
        // row starts, or at the end of the line.
        let run_offsets = layout
            .runs
            .iter()
            .scan(0, |offset, run| {
                let start = *offset;
                *offset += run.glyphs.len();
                Some(start)
            })
            .collect::<Vec<_>>();
        let row_starts = line
            .wrap_boundaries
            .iter()
            .map(|boundary| run_offsets[boundary.run_ix] + boundary.glyph_ix)
            .collect::<Vec<_>>();
        let line_boxes = boxes.len();
        let mut from = 0;
        for (row_in_line, to) in row_starts.iter().copied().chain([glyphs.len()]).enumerate() {
            let start_x = if row_in_line == 0 {
                Pixels::ZERO
            } else {
                glyphs[from].position.x
            };
            let end_x = glyphs
                .get(to)
                .map_or(layout.width, |glyph| glyph.position.x);
            let shift = aligned_row_left(align, align_width, end_x - start_x) - start_x;
            for ix in from..to {
                let glyph = glyphs[ix];
                let right = if ix + 1 < to {
                    glyphs[ix + 1].position.x
                } else {
                    end_x
                };
                boxes.push(GlyphBox {
                    text: line_start + glyph.index..line_start + glyph.index,
                    row: row + row_in_line,
                    left: shift + glyph.position.x,
                    right: shift + right,
                });
            }
            from = to;
        }

        // A glyph draws its text up to where the next glyph's text starts.
        let line_boxes = &mut boxes[line_boxes..];
        line_boxes.sort_by_key(|glyph| glyph.text.start);
        let line_end = line_start + line.len();
        for ix in 0..line_boxes.len() {
            let start = line_boxes[ix].text.start;
            line_boxes[ix].text.end = line_boxes[ix + 1..]
                .iter()
                .map(|glyph| glyph.text.start)
                .find(|next| *next > start)
                .unwrap_or(line_end);
        }

        row += line.wrap_boundaries.len() + 1;
        line_start = line_end + 1;
    }
    boxes
}

fn aligned_row_left(align: TextAlign, align_width: Pixels, width: Pixels) -> Pixels {
    match align {
        TextAlign::Left => Pixels::ZERO,
        TextAlign::Center => (align_width - width) / 2.,
        TextAlign::Right => align_width - width,
    }
}

/// The boxes behind the text of `range`, as a row and the horizontal extent
/// on it, from `glyphs` sorted by the text they draw: every glyph drawing
/// some of that text, joined where they touch on a row.
fn range_boxes(glyphs: &[GlyphBox], range: Range<usize>) -> Vec<(usize, Pixels, Pixels)> {
    let first = glyphs.partition_point(|glyph| glyph.text.end <= range.start);
    let mut hits = glyphs[first..]
        .iter()
        .take_while(|glyph| glyph.text.start < range.end)
        .map(|glyph| (glyph.row, glyph.left, glyph.right))
        .collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        (a.0, a.1)
            .partial_cmp(&(b.0, b.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut boxes: Vec<(usize, Pixels, Pixels)> = Vec::with_capacity(hits.len());
    for (row, left, right) in hits {
        match boxes.last_mut() {
            Some(last) if last.0 == row && left <= last.2 => last.2 = last.2.max(right),
            _ => boxes.push((row, left, right)),
        }
    }
    boxes
}

fn selection_for_multi_click(
    text: &str,
    text_layout: &TextLayout,
    bounds: Bounds<Pixels>,
    pos: Point<Pixels>,
    kind: TextViewMultiClickKind,
) -> Option<std::ops::Range<usize>> {
    if !bounds.contains(&pos) {
        return None;
    }

    let offset = text_layout.index_for_position(pos).ok()?;

    match kind {
        TextViewMultiClickKind::Word => word_range_at(text, offset),
        // Known limitation: a paragraph maps to a single Inline run here. When a
        // paragraph embeds an inline image it is split into multiple Inline runs,
        // so triple-click only selects the run on the clicked side of the image.
        TextViewMultiClickKind::Paragraph | TextViewMultiClickKind::Line => {
            (!text.is_empty()).then_some(0..text.len())
        }
    }
}

/// The carets at the two ends of `range` in `text_layout`, spanning the
/// laid-out `row` when the text is one piece of a row.
fn range_carets(
    text_layout: &TextLayout,
    range: &Range<usize>,
    row: Option<Bounds<Pixels>>,
) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
    let caret = |index| {
        let position = text_layout.position_for_index(index)?;
        Some(match row {
            Some(row) => crate::touch_selection::caret_line_box(
                point(position.x, row.top()),
                row.size.height,
            ),
            None => crate::touch_selection::caret_line_box(position, text_layout.line_height()),
        })
    };
    Some((caret(range.start)?, caret(range.end)?))
}

trait SelectionGeometry {
    fn len(&self) -> usize;
    fn line_height(&self) -> Pixels;
    fn rows_extent(&self) -> (Pixels, Pixels);
    fn position_for_index(&self, index: usize) -> Option<Point<Pixels>>;
}

impl SelectionGeometry for TextLayout {
    fn len(&self) -> usize {
        TextLayout::len(self)
    }
    fn line_height(&self) -> Pixels {
        TextLayout::line_height(self)
    }
    fn rows_extent(&self) -> (Pixels, Pixels) {
        text_rows_extent(self, self.line_height())
    }
    fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        TextLayout::position_for_index(self, index)
    }
}

impl SelectionGeometry for PaintedSelectionLayout {
    fn len(&self) -> usize {
        self.len
    }
    fn line_height(&self) -> Pixels {
        self.line_height
    }
    fn rows_extent(&self) -> (Pixels, Pixels) {
        self.rows_extent()
    }
    fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        self.position_for_index(index)
    }
}

/// Shared by paint and Copy: project the current gesture onto the same shaped
/// UTF-8 text, including rendered inline-code spacing.
fn project_selection(
    text: &str,
    text_layout: &impl SelectionGeometry,
    selection_bounds: Option<Bounds<Pixels>>,
    selection_start: Point<Pixels>,
    selection_end: Point<Pixels>,
) -> Option<Selection> {
    if text.is_empty() {
        return None;
    }
    let line_height = text_layout.line_height();
    let (rows_top, rows_bottom) = match selection_bounds {
        Some(bounds) => (bounds.top(), bounds.bottom()),
        None => text_layout.rows_extent(),
    };
    let band_top = selection_start.y.min(selection_end.y);
    let band_bottom = selection_start.y.max(selection_end.y);
    if rows_bottom <= band_top || rows_top > band_bottom {
        return None;
    }
    if band_top < rows_top && band_bottom >= rows_bottom && text_layout.len() >= text.len() {
        return Some((0..text.len()).into());
    }

    let mut selection: Option<Selection> = None;
    for (offset, c) in text.char_indices() {
        let Some(pos) = text_layout.position_for_index(offset) else {
            continue;
        };
        let next_offset = offset + c.len_utf8();
        let mut char_width = line_height.half();
        if let Some(next_pos) = text_layout.position_for_index(next_offset)
            && next_pos.y == pos.y
        {
            char_width = next_pos.x - pos.x;
        }
        let selection_pos = selection_bounds.map_or(pos, |bounds| point(pos.x, bounds.top()));
        let selection_height = selection_bounds.map_or(line_height, |bounds| bounds.size.height);
        if point_in_text_selection(
            selection_pos,
            char_width,
            selection_start,
            selection_end,
            selection_height,
        ) {
            selection
                .get_or_insert_with(|| Selection::new(offset, offset))
                .end = next_offset;
        }
    }
    selection
}

/// Check if a `pos` is within a `bounds`, considering multi-line selections.
pub(super) fn point_in_text_selection(
    pos: Point<Pixels>,
    char_width: Pixels,
    selection_start: Point<Pixels>,
    selection_end: Point<Pixels>,
    line_height: Pixels,
) -> bool {
    let point_in_line = |point: Point<Pixels>| point.y >= pos.y && point.y < pos.y + line_height;
    let top = selection_start.y.min(selection_end.y);
    let bottom = selection_start.y.max(selection_end.y);
    let x = pos.x + char_width.half();

    // Out of the vertical bounds
    if pos.y + line_height <= top || pos.y > bottom {
        return false;
    }

    // Treat the selection as single-line when both drag points fall within the
    // same rendered line, even if their y coordinates differ inside that line.
    if point_in_line(selection_start) && point_in_line(selection_end) {
        let left = selection_start.x.min(selection_end.x);
        let right = selection_start.x.max(selection_end.x);
        return x >= left && x <= right;
    }

    let (top_point, bottom_point) = if selection_start.y < selection_end.y {
        (selection_start, selection_end)
    } else {
        (selection_end, selection_start)
    };
    let is_top_line = point_in_line(top_point);
    let is_bottom_line = point_in_line(bottom_point);

    if is_top_line {
        return x >= top_point.x;
    } else if is_bottom_line {
        return x <= bottom_point.x;
    } else {
        return true;
    }
}
