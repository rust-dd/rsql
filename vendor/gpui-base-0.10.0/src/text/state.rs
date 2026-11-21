use futures::Stream as _;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
use std::{
    ops::{Range, RangeInclusive},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::Poll,
    time::Duration,
};
#[cfg(target_family = "wasm")]
use web_time::Instant;

use gpui::{
    App, AppContext as _, Bounds, Context, CursorStyle, EntityId, FocusHandle, IntoElement,
    KeyBinding, ListState, ParentElement as _, Pixels, Point, Render, SharedString, Styled as _,
    Task, Window, prelude::FluentBuilder as _, px,
};

use crate::{
    AutoScroll, ElementExt, TextSelection,
    async_util::{Receiver, Sender, unbounded},
    input::{self, SelectAll},
    text::{
        CodeBlockActionsFn, CodeBlockHighlighterFn, LinkClickHandlerFn, LinkFn, MarkdownExtensions,
        RangeHighlight, RangeHighlightError, RenderedText, TableActionsFn, TaskToggleHandlerFn,
        TextViewStyle,
        document::ParsedDocument,
        format,
        node::{self, NodeContext},
        range_highlight::{
            LeafRemap, PendingReveal, RangeHighlightFrame, RenderedIndex, RevealRequest,
        },
        selection_adapter::TextViewSelectionAdapter,
        stream_fade::{StreamFadeTracker, TextViewMotion},
    },
    touch_selection::caret_line_box,
    v_flex,
};

const CONTEXT: &'static str = "TextView";
// Keep coalescing bounded so sustained streams still render intermediate updates.
const MAX_COALESCED_UPDATES_PER_PARSE: usize = 64;
// Preserve exact first-layout height for small documents while bounding the
// amount of source parsed synchronously on the UI thread.
const MAX_SYNC_FULL_REPLACE_BYTES: usize = 4 * 1024;
// Repaint a streamed fade at about 30 fps, not at the display refresh rate.
const STREAM_FADE_TICK: Duration = Duration::from_millis(33);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys(vec![
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", input::Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", input::Copy, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", input::SelectAll, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", input::SelectAll, Some(CONTEXT)),
    ]);
}

/// The content format of the text view.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TextViewFormat {
    /// Markdown view
    Markdown,
    /// HTML view
    Html,
}

/// The format of the text returned by
/// [`TextViewState::selected_text`], which is also what copy writes to the
/// clipboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SelectionFormat {
    /// The rendered text, without any markup.
    #[default]
    Plain,
    /// The source of the selection.
    ///
    /// Select-all returns the original source verbatim, a partial selection is
    /// reconstructed as Markdown from the parsed nodes (e.g. selecting inside
    /// a `**bold**` run yields `**bold**`).
    Source,
    /// The rendered text while the selection stays within one style, the
    /// source as soon as it takes in several.
    ///
    /// A style is everything the source would spell out around the selected
    /// text: the marks of an inline run (emphasis, code, a link's URL) and the
    /// block it sits in (a heading's `#`, a list marker, a code fence). So
    /// selecting inside a `**bold**` run, a link or a code block yields the
    /// text alone, also when the selection reaches exactly up to the run's
    /// boundaries, while a selection across that boundary, into a second
    /// block, or over an inline object copies like [`Self::Source`]. Two blocks
    /// count as two styles even when their marks match, since the source of
    /// either spells out where one ends. Select-all copies the source verbatim.
    SourceAcrossStyles,
}

/// One text element's laid-out vertical extent, reported by `Inline` during
/// prepaint so `TextView` can snap its `max_lines` clip to a whole-line
/// boundary.
#[derive(Clone, Copy)]
pub(super) struct LineSpan {
    pub(super) top: Pixels,
    pub(super) bottom: Pixels,
    pub(super) line_height: Pixels,
}

/// The state of a TextView.
pub struct TextViewState {
    entity_id: EntityId,
    pub(super) focus_handle: FocusHandle,
    pub(super) list_state: ListState,

    /// The bounds of the text view
    bounds: Bounds<Pixels>,

    pub(super) selectable: bool,
    /// The cursor set on the [`super::TextView`] itself; when set, it replaces
    /// the I-beam over selectable text except mid-drag.
    pub(super) hover_cursor: Option<CursorStyle>,
    pub(super) selection_format: SelectionFormat,
    pub(super) scrollable: bool,
    pub(super) max_lines: Option<usize>,
    /// Line spans reported by `Inline` during prepaint (collected only while
    /// [`Self::max_lines`] is set); cleared by `TextView` at each frame start.
    pub(super) line_spans: Arc<Mutex<Vec<LineSpan>>>,
    /// Whether the last painted frame clipped content due to `max_lines`.
    pub(super) clamped: bool,
    pub(super) text_view_style: Arc<TextViewStyle>,
    pub(super) code_block_actions: Option<std::sync::Arc<CodeBlockActionsFn>>,
    pub(super) code_block_highlighter: Option<std::sync::Arc<CodeBlockHighlighterFn>>,
    pub(super) table_actions: Option<std::sync::Arc<TableActionsFn>>,
    pub(super) image_source: Option<std::sync::Arc<super::text_view::ImageSourceFn>>,
    pub(super) link_click_handler: Option<std::sync::Arc<LinkClickHandlerFn>>,
    pub(super) link_hook: Option<std::sync::Arc<LinkFn>>,
    pub(super) link_tooltip: Option<std::sync::Arc<super::text_view::LinkTooltipFn>>,
    pub(super) hovered_link_tooltip: Option<(SharedString, gpui::AnyView)>,
    /// The link under the pointer and the rendering it is in. Two renderings
    /// of one document share a paragraph's state, so its key alone does not
    /// tell them apart; their bounds do.
    pub(super) link_tooltip_target: Option<((usize, Bounds<gpui::Pixels>), SharedString)>,
    pub(super) task_toggle_handler: Option<std::sync::Arc<TaskToggleHandlerFn>>,
    pub(super) markdown_extensions: Arc<MarkdownExtensions>,

    pub(super) is_selecting: bool,
    /// Logical ranges retained across compatible resource or presentation reflow.
    pub(super) preserve_inline_selection: bool,
    multi_click_selection: Option<TextViewMultiClickSelection>,
    selected_text_override: Option<String>,
    select_all: bool,
    pub(super) auto_scroll: AutoScroll,
    pub(super) selection_adapter: TextViewSelectionAdapter,

    pub(super) parsed_content: ParsedContent,
    pub(super) stream_fade: StreamFadeTracker,
    fade_tick: Option<Task<()>>,
    /// Content format (markdown / html), used for bounded synchronous parsing
    /// of small full-replace updates.
    format: TextViewFormat,
    text: String,
    /// The text a `TextView` element last handed over, to recognize the same
    /// string next frame without comparing its bytes.
    element_text: Option<SharedString>,
    revision: usize,
    /// The revision of the update `parsed_content` was committed from.
    committed_revision: usize,
    /// The revision of the last update that replaced the text rather than
    /// appending to it.
    full_update_revision: usize,
    /// The rendered text of `parsed_content`, built when first read.
    rendered_index: Arc<OnceLock<RenderedIndex>>,
    range_highlights: Option<Arc<RangeHighlightFrame>>,
    pub(super) pending_reveal: Option<PendingReveal>,
    pub(super) selection_revision: usize,
    compatible_layout_update: bool,
    layout_text_style: Option<(gpui::TextStyle, Pixels)>,
    parsed_error: Option<SharedString>,
    tx: Sender<UpdateOptions>,
    _parse_task: Task<()>,
    _receive_task: Task<()>,
}

impl TextViewState {
    /// Create a Markdown TextViewState.
    pub fn markdown(text: &str, cx: &mut Context<Self>) -> Self {
        Self::new(TextViewFormat::Markdown, text, cx)
    }

    /// Create a HTML TextViewState.
    pub fn html(text: &str, cx: &mut Context<Self>) -> Self {
        Self::new(TextViewFormat::Html, text, cx)
    }

    /// Create a new TextViewState.
    fn new(format: TextViewFormat, text: &str, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let selection_adapter = TextViewSelectionAdapter::new(cx.entity().downgrade(), cx);

        let (tx, rx) = unbounded::<UpdateOptions>();
        let (tx_result, rx_result) = unbounded::<ParsedUpdate>();
        let _receive_task = cx.spawn({
            async move |weak_self, cx| {
                while let Ok(parsed_update) = rx_result.recv().await {
                    // A result that commits nothing — the background parser
                    // acknowledging a synchronous parse — leaves the state
                    // alone: updating it would tell GPUI Fast's retained
                    // views that read it that it changed.
                    let Ok(commits) =
                        weak_self.read_with(cx, |state, _| state.commits(&parsed_update))
                    else {
                        break;
                    };
                    if !commits {
                        continue;
                    }
                    _ = weak_self.update(cx, |state, cx| {
                        state.commit_parsed_update(parsed_update, cx);
                    });
                }
            }
        });

        let _parse_task = cx.background_spawn(UpdateFuture::new(format, rx, tx_result));

        let mut this = Self {
            entity_id: cx.entity_id(),
            focus_handle,
            bounds: Bounds::default(),
            multi_click_selection: None,
            selected_text_override: None,
            select_all: false,
            selectable: false,
            hover_cursor: None,
            selection_format: SelectionFormat::default(),
            scrollable: false,
            max_lines: None,
            line_spans: Arc::default(),
            clamped: false,
            // Measure all blocks (not just visible ones) so the scrollbar
            // thumb size stays stable. Without this, off-screen blocks count
            // as zero height until scrolled into view, which makes the
            // scrollbar jitter as more blocks get measured during scrolling.
            list_state: ListState::new(0, gpui::ListAlignment::Top, px(1000.)).measure_all(),
            text_view_style: Arc::default(),
            code_block_actions: None,
            code_block_highlighter: None,
            table_actions: None,
            link_click_handler: None,
            link_hook: None,
            link_tooltip: None,
            hovered_link_tooltip: None,
            link_tooltip_target: None,
            task_toggle_handler: None,
            image_source: None,
            markdown_extensions: Arc::default(),
            is_selecting: false,
            preserve_inline_selection: false,
            auto_scroll: AutoScroll::default(),
            selection_adapter,
            parsed_content: Default::default(),
            stream_fade: StreamFadeTracker::default(),
            fade_tick: None,
            format,
            parsed_error: None,
            text: text.to_string(),
            element_text: None,
            revision: 0,
            committed_revision: 0,
            full_update_revision: 0,
            rendered_index: Arc::default(),
            range_highlights: None,
            pending_reveal: None,
            selection_revision: 0,
            compatible_layout_update: false,
            layout_text_style: None,
            tx,
            _parse_task,
            _receive_task,
        };
        this.increment_update(&text, false, cx);
        this
    }

    /// Get the text content.
    pub(crate) fn source(&self) -> SharedString {
        self.parsed_content.document.source.clone()
    }

    /// Set whether the text is selectable, default false.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Set whether the text is selectable, default false.
    pub fn set_selectable(&mut self, selectable: bool, cx: &mut Context<Self>) {
        self.selectable = selectable;
        cx.notify();
    }

    /// Set the [`SelectionFormat`], default is [`SelectionFormat::Plain`].
    pub fn selection_format(mut self, selection_format: SelectionFormat) -> Self {
        self.selection_format = selection_format;
        self
    }

    /// Set the [`SelectionFormat`], default is [`SelectionFormat::Plain`].
    pub fn set_selection_format(
        &mut self,
        selection_format: SelectionFormat,
        cx: &mut Context<Self>,
    ) {
        self.selection_format = selection_format;
        cx.notify();
    }

    /// Set whether the text view scrolls internally, default false.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Set whether the text view scrolls internally, default false.
    pub fn set_scrollable(&mut self, scrollable: bool, cx: &mut Context<Self>) {
        if !scrollable {
            self.reset_selection_and_adapter(cx);
        }
        self.scrollable = scrollable;
        cx.notify();
    }

    /// Whether the last painted frame clipped content because of
    /// [`TextView::max_lines`](crate::text::TextView::max_lines).
    pub fn is_clamped(&self) -> bool {
        self.clamped
    }

    /// Set the text content.
    ///
    /// Markdown that extends the current non-empty text is appended like
    /// [`Self::push_str`], keeping the selection; any other text replaces it.
    /// With a streamed fade-in enabled, extended text fades in; a replacement
    /// shows at once.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.text.as_str() == text {
            return;
        }
        // HTML blocks carry no spans, so an append would parse the delta as a
        // standalone document. After a failed parse the background parser's
        // document lacks that text, so only a full parse resynchronizes it.
        if self.format == TextViewFormat::Markdown
            && self.parsed_error.is_none()
            && !self.text.is_empty()
            && let Some(delta) = text.strip_prefix(self.text.as_str())
        {
            self.push_str(delta, cx);
            return;
        }
        self.stream_fade.note_replace();

        self.text.clear();
        self.text.push_str(text);
        self.parsed_error = None;
        self.increment_update(text, false, cx);
    }

    /// [`Self::set_text`] for the text a `TextView` element hands over every
    /// frame: the string it handed over last time (the same allocation, not
    /// merely equal bytes) is recognized without a comparison, as long as the
    /// state still holds a text of its length.
    pub(super) fn set_element_text(&mut self, text: &SharedString, cx: &mut Context<Self>) {
        let same_string = self.element_text.as_ref().is_some_and(|current| {
            current.as_ptr() == text.as_ptr() && current.len() == text.len()
        });
        if same_string && self.text.len() == text.len() {
            return;
        }
        self.element_text = Some(text.clone());
        self.set_text(text, cx);
    }

    /// Append partial text content to the existing text.
    pub fn push_str(&mut self, new_text: &str, cx: &mut Context<Self>) {
        if new_text.is_empty() {
            return;
        }
        self.stream_fade.note_extend(self.text.len());
        self.text.push_str(new_text);
        self.increment_update(new_text, true, cx);
    }

    /// Set the motion policy; see [`TextViewMotion`].
    pub fn motion(mut self, motion: TextViewMotion) -> Self {
        self.set_motion(motion);
        self
    }

    /// Set the motion policy; see [`TextViewMotion`].
    pub fn set_motion(&mut self, motion: TextViewMotion) {
        self.stream_fade.set_motion(motion);
    }

    pub(crate) fn set_markdown_extensions(
        &mut self,
        markdown_extensions: Arc<MarkdownExtensions>,
        cx: &mut Context<Self>,
    ) {
        if self.markdown_extensions.revision() == markdown_extensions.revision() {
            return;
        }

        let parser_configuration_changed = !self
            .markdown_extensions
            .has_same_parser_configuration(&markdown_extensions);
        self.markdown_extensions = markdown_extensions;
        if parser_configuration_changed && self.format == TextViewFormat::Markdown {
            let text = self.text.clone();
            self.increment_update(&text, false, cx);
        }
    }

    /// Return the selected text, in the view's [`SelectionFormat`].
    pub fn selected_text(&self) -> String {
        self.selected_text_in(None)
    }

    /// Return the original Markdown source byte range corresponding to the
    /// rendered selection.
    ///
    /// The range addresses the source passed to this Markdown TextView. It is
    /// derived from parser positions retained by the rendered inline nodes, so
    /// identical rendered text maps to the occurrence that was actually
    /// selected. The result is one contiguous source range, so it includes any
    /// Markdown delimiters between the selected rendered endpoints. HTML views
    /// and selections without an exact source mapping return `None`. Select-all
    /// in a Markdown view returns the full source range.
    pub fn selected_source_range(&self) -> Option<Range<usize>> {
        if self.format != TextViewFormat::Markdown {
            return None;
        }
        if self.select_all {
            return Some(0..self.source().len());
        }
        self.parsed_content.document.selected_source_range()
    }

    /// The format to copy in, which is [`SelectionFormat::Plain`] whenever the
    /// requested one cannot be produced.
    ///
    /// Only a Markdown view can return source. Reconstructing HTML would mean
    /// spelling every attribute back out — a mark's color, an image's
    /// dimensions, a cell's alignment — with a new way to lose one at each
    /// step, and html5ever records no source offsets to fall back on (it
    /// reports only line numbers), so there is no original text to copy from
    /// either.
    fn effective_format(&self) -> SelectionFormat {
        match self.format {
            TextViewFormat::Markdown => self.selection_format,
            TextViewFormat::Html => SelectionFormat::Plain,
        }
    }

    /// A pending full replacement leaves the previous document on screen, but
    /// that document must no longer supply clipboard content.
    pub(super) fn has_current_copy_document(&self) -> bool {
        self.committed_revision >= self.full_update_revision
    }

    /// Return the selected text, with `blocks` bounding which top-level blocks
    /// the selection covers.
    ///
    /// The range comes from the selection endpoints, which know their block
    /// even after it scrolls out of view; see
    /// [`ParsedDocument::selected_text`](crate::text::document::ParsedDocument).
    pub(super) fn selected_text_in(&self, blocks: Option<RangeInclusive<usize>>) -> String {
        let format = self.effective_format();

        if self.select_all {
            if format != SelectionFormat::Plain {
                return self.source().to_string();
            }

            return self.parsed_content.document.text();
        }

        // A multi-click stores the plain text it selected, which is a shortcut
        // past the block walk. Source mode cannot take it: the word it stored
        // has lost its markup. The click also set the inline selection it came
        // from, so the walk reconstructs the same range with the markup intact.
        if format == SelectionFormat::Plain
            && let Some(text) = &self.selected_text_override
        {
            return text.clone();
        }

        self.parsed_content.document.selected_text(format, blocks)
    }

    /// Force a full re-measure of the block list after the document has been
    /// replaced.
    ///
    /// `Document::render_root` only calls `ListState::reset` when the block
    /// *count* changes, and the full-measure pass enabled by `measure_all` is
    /// a one-shot latch that only `reset`, `remeasure_items`, or a width
    /// change re-arms. Replacing a document with one that happens to have the
    /// same number of blocks therefore leaves every cached height belonging to
    /// the *previous* document. The list summary height stays wrong, and since
    /// the wheel clamps against that summary, the blocks past the false bottom
    /// can never scroll into view to be re-measured -- the clamp seals itself.
    ///
    /// `remeasure_items` re-arms the latch and marks the items unmeasured
    /// while keeping their old sizes as hints, so the scrollbar does not
    /// collapse in the frame before the next layout measures the real heights.
    fn invalidate_measured_heights(&self) {
        let count = self.list_state.item_count();
        if count > 0 {
            self.list_state.remeasure_items(0..count);
        }
    }

    /// Remeasure inline resources after their prepared content or metrics change.
    ///
    /// Call from the resource owner's completion handler, through a weak entity
    /// when the task can outlive this view. This does not reparse the document.
    /// Existing logical selection is retained until the next selection gesture.
    pub fn invalidate_inline_layout(&mut self, cx: &mut Context<Self>) {
        self.preserve_selection_for_reflow();
        cx.notify();
    }

    pub(super) fn preserve_selection_for_reflow(&mut self) {
        self.preserve_inline_selection = true;
        self.compatible_layout_update = true;
        self.invalidate_measured_heights();
    }

    fn increment_update(&mut self, text: &str, append: bool, cx: &mut Context<Self>) {
        self.revision += 1;
        if !append {
            self.full_update_revision = self.revision;
            self.selection_revision = self.selection_revision.wrapping_add(1);
            // Old native layouts must not project a still-active gesture back
            // into replaced content, including while a large parse is pending.
            self.selection_adapter.clear_copy_projections(cx);
            self.parsed_content.document.clear_selection();
        }
        let parse_synchronously = !append && text.len() <= MAX_SYNC_FULL_REPLACE_BYTES;
        let update_options = UpdateOptions {
            revision: self.revision,
            append,
            mode: if append {
                ParseMode::Compatible
            } else if parse_synchronously {
                ParseMode::BaselineAck
            } else {
                ParseMode::Replace
            },
            pending_text: text.to_string(),
            markdown_extensions: self.markdown_extensions.clone(),
        };

        // Keep small full replacements synchronous so their first layout has
        // the exact content height. Larger replacements use the existing
        // background parser, bounding synchronous parser input on the UI thread.
        if parse_synchronously {
            match parse_content(self.format, ParsedContent::default(), &update_options) {
                Ok(content) => {
                    self.reconcile_range_highlights(&content.document, self.revision, false);
                    self.stream_fade.record(
                        &self.parsed_content.document,
                        &content.document,
                        Instant::now(),
                    );
                    self.parsed_content = content;
                    self.parsed_error = None;
                    self.invalidate_measured_heights();
                    if !self.is_selecting {
                        self.reset_selection_and_adapter(cx);
                    }
                }
                Err(err) => {
                    self.stream_fade.discard_pending();
                    self.parsed_error = Some(err);
                }
            }
            // Keep the background parser's accumulated document in sync so a
            // later append extends this baseline instead of parsing the delta
            // as a standalone document.
            _ = self.tx.try_send(update_options);
            cx.notify();
            return;
        }

        _ = self.tx.try_send(update_options);
    }

    /// Whether [`Self::commit_parsed_update`] commits `parsed_update`: it is
    /// not older than what is committed or than the last replacement, nor
    /// the background parser acknowledging a synchronous parse.
    fn commits(&self, parsed_update: &ParsedUpdate) -> bool {
        if parsed_update.revision < self.full_update_revision
            || parsed_update.revision <= self.committed_revision
        {
            return false;
        }
        debug_assert!(!parsed_update.baseline_ack || parsed_update.full_parse);
        !parsed_update.baseline_ack
    }

    /// Commit a result of the background parser.
    ///
    /// A stream that appends faster than a parse completes has always moved
    /// past the revision the result was parsed from, so only discarding
    /// results of an older revision would show nothing until the stream
    /// stops. A result parsed since the text was last replaced is a prefix of
    /// the current text and is committed; one from before that replacement,
    /// or older than what is already committed, is discarded.
    fn commit_parsed_update(&mut self, parsed_update: ParsedUpdate, cx: &mut Context<Self>) {
        if !self.commits(&parsed_update) {
            return;
        }

        match parsed_update.result {
            Ok(content) => {
                let append = parsed_update.selection_compatible && !parsed_update.full_parse;
                if append && self.full_update_revision <= self.committed_revision {
                    self.splice_appended_blocks(&content.document);
                }
                self.reconcile_range_highlights(&content.document, parsed_update.revision, append);
                self.stream_fade.record(
                    &self.parsed_content.document,
                    &content.document,
                    Instant::now(),
                );
                // This result may cover only part of the queued appends.
                // Keep the uncommitted tail pending from this document's end,
                // rather than consuming its fade with the earlier chunk.
                if parsed_update.revision < self.revision {
                    self.stream_fade.note_extend(content.document.source.len());
                }
                self.parsed_content = content;
                self.parsed_error = None;
                self.compatible_layout_update = parsed_update.selection_compatible;
                if parsed_update.full_parse {
                    self.invalidate_measured_heights();
                }
            }
            Err(err) => {
                self.stream_fade.discard_pending();
                self.parsed_error = Some(err);
            }
        }
        // Don't interrupt an active drag-selection; the stored
        // positions remain valid for append-only updates and will
        // self-correct on the next mouse-move event.
        if !parsed_update.selection_compatible && !self.is_selecting {
            self.reset_selection_and_adapter(cx);
        }
        cx.notify();
    }

    /// The text this view renders, which [`RangeHighlight`] ranges index.
    ///
    /// This is the string plain copy produces, as of the last parse that
    /// landed; text set since then is not in it until its parse lands.
    pub fn rendered_text(&self) -> RenderedText {
        RenderedText::new(
            self.entity_id,
            self.committed_revision,
            self.parsed_content.document.clone(),
            self.rendered_index.clone(),
        )
    }

    /// Replace the range highlights, whose ranges index the current rendered text.
    ///
    /// Compute ranges from the current [`Self::rendered_text`] and set them in
    /// the same update. Search the new text again after its content changes.
    /// Ranges of the Markdown source, such as [`Self::selected_source_range`]
    /// returns, convert with [`RenderedText::range_for_source`].
    /// A range crossing blocks paints in both, skipping their separator.
    /// Text outside every block (separators, custom blocks, HTML blocks, and
    /// inline objects) is left unpainted. Any invalid range rejects the set.
    ///
    /// Highlights follow unchanged text through updates. Text appended while
    /// streaming keeps earlier highlights; after a table edit, cells in and
    /// after the edited row lose theirs because cells are known by position.
    /// Text backgrounds such as `<mark>` paint over a highlight; inline code
    /// backgrounds paint under it.
    pub fn set_range_highlights(
        &mut self,
        highlights: impl IntoIterator<Item = RangeHighlight>,
        cx: &mut Context<Self>,
    ) -> Result<(), RangeHighlightError> {
        if self.format != TextViewFormat::Markdown {
            return Err(RangeHighlightError::Unsupported);
        }
        let text = self.rendered_text();
        self.range_highlights = RangeHighlightFrame::new(&text, highlights)?.map(Arc::new);
        cx.notify();
        Ok(())
    }

    /// Scroll the line `range` starts on into view, `range` indexing the
    /// current rendered text, as with [`Self::set_range_highlights`].
    ///
    /// A [`Self::scrollable`] view scrolls itself. Otherwise the nearest
    /// enclosing `gpui::list` scrolls, as long as the row holding the view is
    /// laid out, so scroll to that row first when it may be off screen. Any
    /// other scroll container scrolls through
    /// [`TextView::on_reveal`](crate::text::TextView::on_reveal). An empty
    /// range reveals the line of its position; a range in text that belongs
    /// to no block's text, such as a custom block's, scrolls its whole block
    /// into a scrollable view.
    ///
    /// Only the latest reveal is carried out. It follows the content as
    /// [range highlights](Self::set_range_highlights) do, and it is dropped
    /// when its text changes, when the view clamps its lines, or when it
    /// cannot be shown within a second, so it never scrolls long after it
    /// was asked for. A whole block taller than the view, scrolled to from
    /// below, shows its end.
    ///
    /// Revealing is best effort: `Ok(())` means the range is valid for the
    /// current text and the request was taken, not that the view has
    /// scrolled. A dropped request is not reported.
    pub fn reveal_range(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Result<(), RangeHighlightError> {
        if self.format != TextViewFormat::Markdown {
            return Err(RangeHighlightError::Unsupported);
        }
        let text = self.rendered_text();
        if text.is_empty() && range == (0..0) {
            // Nothing to reveal in an empty view.
            self.pending_reveal = None;
            return Ok(());
        }
        let now = cx.background_executor().now();
        self.pending_reveal = Some(
            PendingReveal::new(&text, &range, now).ok_or(RangeHighlightError::InvalidRange(0))?,
        );
        cx.notify();
        Ok(())
    }

    /// Remove all range highlights.
    pub fn clear_range_highlights(&mut self, cx: &mut Context<Self>) {
        if self.range_highlights.take().is_some() {
            cx.notify();
        }
    }

    /// Carry the range highlights over to `new`, the document the update of
    /// `revision` parsed, before it replaces the current one.
    ///
    /// `append` is whether that update parsed only appended text onto the
    /// document before it. Only when no update has replaced the text since
    /// the current document was committed was that document the current one,
    /// so only then do the blocks before its last stay unchanged.
    fn reconcile_range_highlights(&mut self, new: &ParsedDocument, revision: usize, append: bool) {
        let tail_only = append && self.full_update_revision <= self.committed_revision;
        self.committed_revision = revision;
        self.rendered_index = Arc::default();
        if self.range_highlights.is_some() || self.pending_reveal.is_some() {
            let remap = LeafRemap::new(&self.parsed_content.document, new, tail_only);
            self.range_highlights = self
                .range_highlights
                .take()
                .and_then(|highlights| highlights.remap(&remap))
                .map(Arc::new);
            self.pending_reveal = self
                .pending_reveal
                .take()
                .and_then(|reveal| reveal.remap(&remap));
        }
    }

    /// Tell the scrollable list which blocks an append replaced, before `new`,
    /// the appended document, replaces the current one.
    ///
    /// An append parses only the current document's last block again, so the
    /// blocks before it keep their spans and their measured heights. Without
    /// this, `render_root` resets the list whenever the block count changes,
    /// which scrolled a streaming view back to the top and measured every
    /// block again on each new block. Only a grown block count is handled
    /// here; anything else is left to that reset.
    fn splice_appended_blocks(&self, new: &ParsedDocument) {
        let old = &self.parsed_content.document.blocks;
        let (old_count, new_count) = (old.len(), new.blocks.len());
        if old_count == 0 || new_count <= old_count || self.list_state.item_count() != old_count {
            return;
        }

        // The last block is always measured again: it was parsed again, and
        // it is no longer the last one.
        let unchanged = old
            .iter()
            .zip(new.blocks.iter())
            .take(old_count - 1)
            .take_while(|(old_block, new_block)| {
                old_block
                    .span()
                    .is_some_and(|span| new_block.span() == Some(span))
            })
            .count();
        // Keeps the replaced blocks' heights as hints and the scroll offset
        // inside them, and has the next layout measure only the blocks that
        // are not measured.
        self.list_state.remeasure_items(unchanged..old_count);
        self.list_state
            .splice(old_count..old_count, new_count - old_count);
    }

    /// Save bounds and unselect if bounds changed.
    pub(super) fn update_bounds(&mut self, bounds: Bounds<Pixels>, _cx: &mut App) {
        self.bounds = bounds;
    }

    /// The index of the top-level block at `content_y`, in this view's content
    /// coordinates (the same space the base selection endpoint stores its point in).
    ///
    /// Only laid-out blocks can be located, which is enough for a selection
    /// endpoint: the user can only put one where they can see it. Returns
    /// `None` for a view that is not virtualized, where every block paints and
    /// the range is not needed.
    pub(super) fn block_ix_at(&self, content_y: Pixels) -> Option<usize> {
        if !self.scrollable {
            return None;
        }

        let origin = self.bounds.origin.y + self.scroll_offset().y;
        let count = self.list_state.item_count();
        let mut ix = self.list_state.logical_scroll_top().item_ix;
        while ix < count {
            let bounds = self.list_state.bounds_for_item(ix)?;
            if content_y < bounds.bottom() - origin {
                return Some(ix);
            }
            ix += 1;
        }

        count.checked_sub(1)
    }

    #[doc(hidden)]
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    #[doc(hidden)]
    pub fn list_state(&self) -> &ListState {
        &self.list_state
    }

    #[doc(hidden)]
    pub fn is_selecting(&self) -> bool {
        self.is_selecting
    }

    #[doc(hidden)]
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Whether this view has a view-local selection (select-all, multi-click, or override),
    /// independent of the window-level selection.
    pub(super) fn has_view_selection(&self) -> bool {
        self.select_all
            || self.multi_click_selection.is_some()
            || self.selected_text_override.is_some()
    }

    pub(super) fn stop_auto_scroll(&mut self) {
        self.auto_scroll.stop();
    }

    pub(super) fn reset_selection(&mut self) {
        self.preserve_inline_selection = false;
        self.multi_click_selection = None;
        self.selected_text_override = None;
        self.select_all = false;
        self.is_selecting = false;
        self.auto_scroll.stop();
        // Clear the inline selection state synchronously, so offscreen
        // (virtualized) views that won't repaint don't leak stale selection
        // text into a new cross-view copy.
        self.parsed_content.document.clear_selection();
    }

    fn reset_selection_and_adapter(&mut self, cx: &mut App) {
        self.reset_selection();
        self.selection_adapter.set_local_selection(false, cx);
    }

    /// Clear the current text selection.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.reset_selection_and_adapter(cx);
        cx.notify();
    }

    pub(super) fn scroll_offset(&self) -> Point<Pixels> {
        if self.scrollable {
            self.list_state.scroll_px_offset_for_scrollbar()
        } else {
            Point::default()
        }
    }

    /// Select all rendered text in this view.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.multi_click_selection = None;
        self.selected_text_override = None;
        self.select_all = true;
        self.is_selecting = false;
        self.auto_scroll.stop();
        self.selection_adapter.set_local_selection(true, cx);
        cx.notify();
    }

    /// Selects what a multi-click at `pos` selected, from the caret `start`
    /// to the caret `end` in window coordinates. A drag from the click
    /// extends it by the same unit.
    pub(crate) fn set_multi_click_selection(
        &mut self,
        pos: Point<Pixels>,
        kind: TextViewMultiClickKind,
        selected_text: String,
        (start, end): (Bounds<Pixels>, Bounds<Pixels>),
        cx: &mut App,
    ) {
        self.preserve_inline_selection = false;
        let scroll_offset = self.scroll_offset();
        let pos = pos - self.bounds.origin - scroll_offset;
        self.multi_click_selection = Some(TextViewMultiClickSelection {
            pos,
            kind,
            line_bounds: None,
        });
        self.selected_text_override = Some(selected_text);
        self.select_all = false;
        self.is_selecting = false;
        self.auto_scroll.stop();
        self.selection_adapter.set_local_selection(true, cx);
        let click_count = match kind {
            TextViewMultiClickKind::Word => 2,
            TextViewMultiClickKind::Paragraph | TextViewMultiClickKind::Line => 3,
        };
        self.selection_adapter
            .hold_multi_click(start, end, click_count, cx);
    }

    /// Hands a multi-click selection over to the window selection, which a
    /// drag from the click has taken beyond what the click selected.
    pub(super) fn end_multi_click_selection(&mut self, cx: &mut App) {
        if self.multi_click_selection.take().is_some() {
            self.selected_text_override = None;
            self.selection_adapter.set_local_selection(false, cx);
        }
    }

    /// Selects every inline in `rows.bounds`, as a triple click on a
    /// paragraph laid out as an inline flow does: from the start of its first
    /// row to the end of its last row.
    pub(crate) fn set_multi_click_line(&mut self, rows: ParagraphRows, cx: &mut App) {
        let bounds = rows.bounds;
        self.set_multi_click_selection(
            bounds.center(),
            TextViewMultiClickKind::Line,
            String::new(),
            (
                caret_line_box(bounds.origin, rows.first_row_height),
                caret_line_box(
                    gpui::point(bounds.right(), bounds.bottom() - rows.last_row_height),
                    rows.last_row_height,
                ),
            ),
            cx,
        );
        self.selected_text_override = None;
        let offset = self.bounds.origin + self.scroll_offset();
        if let Some(selection) = self.multi_click_selection.as_mut() {
            selection.line_bounds = Some(Bounds::new(bounds.origin - offset, bounds.size));
        }
    }

    pub(super) fn set_auto_scroll(&mut self, delta: Option<Pixels>, cx: &mut Context<Self>) {
        self.auto_scroll.set(delta, cx, |delta, state, cx| {
            state.list_state.scroll_by(delta);
            cx.notify();
        });
    }

    /// Return the window selection (anchor, cursor) in window coordinates if
    /// this view participates in it.
    ///
    /// Single-view fast path: when both endpoints are anchored inside one
    /// TextView, only that view participates (identical to the previous
    /// per-view behavior).
    pub(crate) fn selection_points(&self, cx: &App) -> Option<(Point<Pixels>, Point<Pixels>)> {
        if !self.selectable {
            return None;
        }
        self.selection_adapter.selection_points(cx)
    }

    /// [`Self::selection_points`] for the text that paints now: a drag that
    /// started in a table cell selects only in that cell. Call it only while
    /// painting, so the cell the text is in is known.
    pub(crate) fn painted_selection_points(
        &self,
        cx: &App,
    ) -> Option<(Point<Pixels>, Point<Pixels>)> {
        if !self.selectable {
            return None;
        }
        self.selection_adapter.painted_selection_points(cx)
    }

    pub(crate) fn has_selection(&self, cx: &App) -> bool {
        self.has_view_selection() || self.selection_points(cx).is_some()
    }

    pub(super) fn on_action_select_all(
        &mut self,
        _: &SelectAll,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selectable {
            cx.propagate();
            return;
        }

        self.select_all(cx);
    }

    pub(crate) fn is_selectable(&self) -> bool {
        self.selectable
    }

    /// Whether selectable text should show the I-beam now: always, unless the
    /// [`super::TextView`] sets its own cursor, then only mid-drag.
    pub(crate) fn shows_text_cursor(&self) -> bool {
        self.hover_cursor.is_none() || self.is_selecting
    }

    pub(crate) fn is_all_selected(&self) -> bool {
        self.select_all
    }

    pub(crate) fn multi_click_selection(&self) -> Option<TextViewMultiClickSelection> {
        let scroll_offset = self.scroll_offset();
        self.multi_click_selection.map(|selection| {
            let pos = selection.pos + scroll_offset + self.bounds.origin;
            let line_bounds = selection.line_bounds.map(|bounds| {
                Bounds::new(
                    bounds.origin + scroll_offset + self.bounds.origin,
                    bounds.size,
                )
            });
            TextViewMultiClickSelection {
                pos,
                line_bounds,
                ..selection
            }
        })
    }
}

/// The rows a triple click selects as one paragraph: their union, and the
/// heights of the first and last rows for the selection's carets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParagraphRows {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) first_row_height: Pixels,
    pub(crate) last_row_height: Pixels,
}

impl ParagraphRows {
    /// One row on its own.
    pub(crate) fn single(bounds: Bounds<Pixels>) -> Self {
        Self {
            bounds,
            first_row_height: bounds.size.height,
            last_row_height: bounds.size.height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextViewMultiClickSelection {
    pub(crate) pos: Point<Pixels>,
    pub(crate) kind: TextViewMultiClickKind,
    pub(crate) line_bounds: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextViewMultiClickKind {
    Word,
    Paragraph,
    Line,
}

impl TextViewState {
    /// Starts a frame of the pending reveal: the line to hand to rendering,
    /// and the block a scrollable view has to scroll to first, because it is
    /// off screen or the reveal is of a whole block.
    fn reveal_frame(
        &mut self,
        now: Instant,
        window: &mut Window,
    ) -> (Option<RevealRequest>, Option<usize>) {
        if self.max_lines.is_some()
            || self
                .pending_reveal
                .as_ref()
                .is_some_and(|reveal| reveal.is_expired(now))
        {
            self.pending_reveal = None;
        }
        let Some(pending) = &self.pending_reveal else {
            return (None, None);
        };

        let block_ix = pending.block_ix(&self.parsed_content.document);
        let block_off_screen = |ix: usize| {
            let viewport = self.list_state.viewport_bounds();
            self.list_state.bounds_for_item(ix).is_none_or(|item| {
                item.bottom() <= viewport.top() || item.top() >= viewport.bottom()
            })
        };
        let reveal_block = block_ix.filter(|ix| {
            self.scrollable
                && (pending.is_block() || !pending.was_laid_out())
                && block_off_screen(*ix)
        });
        let request = pending.request();
        if pending.is_block() {
            // A block has no line to wait for.
            self.pending_reveal = None;
        } else {
            window.request_animation_frame();
        }
        (request, reveal_block)
    }
}

impl Render for TextViewState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let typography = (window.text_style(), window.rem_size());
        let body_font_size = typography.0.font_size.to_pixels(typography.1);
        if self
            .layout_text_style
            .as_ref()
            .is_some_and(|previous| previous != &typography)
        {
            // ListState invalidates its cached rows for width changes, but does
            // not know that an inherited font or rem change affects offscreen
            // inline metrics. Retain logical selections just as for resources.
            self.preserve_selection_for_reflow();
        }
        self.layout_text_style = Some(typography);
        let state = cx.entity();
        let stream_fade = self.stream_fade.frame(Instant::now(), cx.reduce_motion());
        if stream_fade.is_some() && self.fade_tick.is_none() {
            self.fade_tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(STREAM_FADE_TICK).await;
                _ = this.update(cx, |state, cx| {
                    state.fade_tick = None;
                    cx.notify();
                });
            }));
        }
        let (reveal, reveal_block) = self.reveal_frame(cx.background_executor().now(), window);
        // Built every frame, so everything in it is shared, not copied.
        let node_cx = NodeContext {
            offset: self.parsed_content.node_cx.offset,
            link_refs: self.parsed_content.node_cx.link_refs.clone(),
            style: self.text_view_style.clone(),
            code_block_actions: self.code_block_actions.clone(),
            code_block_highlighter: self.code_block_highlighter.clone(),
            table_actions: self.table_actions.clone(),
            link_click_handler: self.link_click_handler.clone(),
            link_hook: self.link_hook.clone(),
            task_toggle_handler: self.task_toggle_handler.clone(),
            image_source: self.image_source.clone(),
            markdown_extensions: self.markdown_extensions.clone(),
            stream_fade,
            range_highlights: self.range_highlights.clone(),
            foregrounds: Default::default(),
            reveal,
            body_font_size,
        };

        let content = v_flex()
            .w_full()
            // Clamped content must keep its natural height: stretching it to
            // the capped box would hide the overflow the clamp has to measure.
            .when(self.max_lines.is_none(), |this| this.h_full())
            .map(|this| match &self.parsed_error {
                None => this.child(self.parsed_content.document.render_root(
                    if self.scrollable {
                        Some(self.list_state.clone())
                    } else {
                        None
                    },
                    &node_cx,
                    window,
                    cx,
                )),
                Some(err) => this.child(
                    v_flex()
                        .gap_1()
                        .child("Failed to parse content")
                        .child(err.to_string()),
                ),
            })
            .on_prepaint(move |bounds, window, cx| {
                let (
                    size_changed,
                    selection_involves_view,
                    has_selection_snapshot,
                    is_selecting,
                    compatible_layout_update,
                    preserve_width_selection,
                ) = {
                    let state = state.read(cx);
                    let has_selection_snapshot = state.selection_adapter.has_selection_snapshot(cx);
                    let preserve_width_selection = !state.is_selecting
                        && state.bounds().size.width != bounds.size.width
                        && (has_selection_snapshot || state.has_view_selection());
                    (
                        state.bounds().size != bounds.size,
                        state.selection_adapter.is_part_of_window_selection(cx),
                        has_selection_snapshot,
                        state.is_selecting,
                        // A measurement prepaint must not consume compatibility
                        // before the same entity's visible column prepaints.
                        state.compatible_layout_update
                            || preserve_width_selection
                            || state.preserve_inline_selection,
                        preserve_width_selection,
                    )
                };
                let mut revision_changed = false;
                state.update(cx, |state, cx| {
                    revision_changed = state
                        .selection_adapter
                        .update_layout_revision(state.selection_revision, state.is_selecting);
                    if preserve_width_selection {
                        // ListState already remeasures width changes. Arm logical
                        // retention before inline paint without invalidating rows again.
                        state.preserve_inline_selection = true;
                    }
                    state.update_bounds(bounds, cx);
                    state.compatible_layout_update = false;
                });
                if !is_selecting
                    && ((size_changed && selection_involves_view && !compatible_layout_update)
                        || (revision_changed && has_selection_snapshot))
                {
                    TextSelection::clear(window, cx);
                }
            });
        // After `render_root`, which resets the list when the block count
        // changed other than by an append, and with it the scroll position.
        if let Some(block_ix) = reveal_block {
            self.list_state.scroll_to_reveal_item(block_ix);
        }
        content
    }
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct ParsedContent {
    pub(crate) document: ParsedDocument,
    pub(crate) node_cx: node::NodeContext,
}

struct UpdateFuture {
    format: TextViewFormat,
    content: ParsedContent,
    rx: Pin<Box<Receiver<UpdateOptions>>>,
    tx_result: Sender<ParsedUpdate>,
}

impl UpdateFuture {
    fn new(
        format: TextViewFormat,
        rx: Receiver<UpdateOptions>,
        tx_result: Sender<ParsedUpdate>,
    ) -> Self {
        Self {
            format,
            content: Default::default(),
            rx: Box::pin(rx),
            tx_result,
        }
    }
}

impl Future for UpdateFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.rx.as_mut().poll_next(cx) {
                Poll::Ready(Some(mut options)) => {
                    let hit_coalesce_budget =
                        merge_pending_options(&mut options, self.rx.as_ref().get_ref());

                    let res = parse_content(self.format, self.content.clone(), &options);
                    if let Ok(content) = &res {
                        self.content = content.clone();
                    }
                    _ = self.tx_result.try_send(ParsedUpdate {
                        revision: options.revision,
                        full_parse: !options.append,
                        selection_compatible: options.mode == ParseMode::Compatible,
                        baseline_ack: options.mode == ParseMode::BaselineAck,
                        result: res,
                    });
                    if hit_coalesce_budget {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[derive(Clone)]
struct UpdateOptions {
    revision: usize,
    pending_text: String,
    append: bool,
    mode: ParseMode,
    markdown_extensions: Arc<MarkdownExtensions>,
}

impl UpdateOptions {
    fn merge(&mut self, next: UpdateOptions) {
        if next.append {
            self.pending_text.push_str(&next.pending_text);
            self.revision = next.revision;
            if self.mode != ParseMode::Replace {
                self.mode = ParseMode::Compatible;
            }
        } else {
            *self = next;
        }
    }
}

struct ParsedUpdate {
    revision: usize,
    full_parse: bool,
    selection_compatible: bool,
    baseline_ack: bool,
    result: Result<ParsedContent, SharedString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseMode {
    BaselineAck,
    Replace,
    Compatible,
}

fn merge_pending_options(options: &mut UpdateOptions, rx: &Receiver<UpdateOptions>) -> bool {
    let mut update_count = 1;

    while update_count < MAX_COALESCED_UPDATES_PER_PARSE {
        match rx.try_recv() {
            Ok(next_options) => {
                options.merge(next_options);
                update_count += 1;
            }
            Err(_) => return false,
        }
    }

    true
}

fn parse_content(
    format: TextViewFormat,
    mut content: ParsedContent,
    options: &UpdateOptions,
) -> Result<ParsedContent, SharedString> {
    let mut node_cx = NodeContext {
        markdown_extensions: options.markdown_extensions.clone(),
        ..NodeContext::default()
    };

    // Re-parse the last block together with the appended text, so a block the
    // new text continues (an unclosed list, a fenced code block) is not split
    // in two. A block without a span cannot be located in `source` — the HTML
    // parser never records spans — so it is left in place and only the
    // appended text is parsed, positioned at the end of the current source.
    let last_span = options
        .append
        .then(|| {
            content
                .document
                .blocks
                .last()
                .and_then(|block| block.span())
        })
        .flatten();

    let mut source = String::new();
    if let Some(span) = last_span {
        Arc::make_mut(&mut content.document.blocks).pop();
        node_cx.offset = span.start;
        source.push_str(&content.document.source[span.start..]);
        source.push_str(&options.pending_text);
    } else {
        if options.append {
            node_cx.offset = content.document.source.len();
        }
        source.push_str(&options.pending_text);
    }

    let new_document = match format {
        TextViewFormat::Markdown => format::markdown::parse(&source, &mut node_cx),
        TextViewFormat::Html => format::html::parse(&source, &mut node_cx),
    }?;

    if options.append {
        content.document.source =
            format!("{}{}", content.document.source, options.pending_text).into();
        Arc::make_mut(&mut content.document.blocks)
            .extend(Arc::unwrap_or_clone(new_document.blocks));
    } else {
        content.document = new_document;
    }

    Ok(content)
}
