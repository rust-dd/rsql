use std::{ops::Range, rc::Rc, sync::Arc};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, Bounds, ClickEvent, ContentMask, Element, ElementId, Entity, Global,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, InteractiveElement, IntoElement,
    LayoutId, MouseButton, ParentElement, Pixels, Refineable as _, SharedString, StyleRefinement,
    Styled, Window, div, point, px,
};

use crate::StyledExt;
use crate::text::TextViewFormat;
use crate::text::markdown_ext::{MarkdownExtensions, MarkdownNode, MarkdownPlugin};
use crate::text::node::{CodeBlock, TableData};
use crate::text::range_highlight::{PendingReveal, RevealProgress};
use crate::text::state::{LineSpan, SelectionFormat, TextViewState};
use crate::text::stream_fade::TextViewMotion;
use crate::{GlobalState, TextSelection, text::TextViewStyle};

/// Type for code block actions generator function.
pub(crate) type CodeBlockActionsFn =
    dyn Fn(&CodeBlock, &mut Window, &mut App) -> AnyElement + Send + Sync;

pub(crate) type CodeBlockHighlighterFn =
    dyn Fn(&CodeBlock) -> Vec<(Range<usize>, gpui::HighlightStyle)> + Send + Sync;

/// Application-wide defaults for TextViews that do not provide explicit
/// presentation or syntax-highlighting overrides.
#[derive(Clone, Default)]
pub struct TextViewDefaults {
    style: Option<TextViewStyle>,
    code_block_highlighter: Option<Arc<CodeBlockHighlighterFn>>,
    inherit_text_color: bool,
}

impl Global for TextViewDefaults {}

impl TextViewDefaults {
    /// Creates defaults that leave every text view as Base renders it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the style every text view starts from.
    pub fn with_style(mut self, style: TextViewStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The style every text view starts from, when set.
    pub(crate) fn style(&self) -> Option<&TextViewStyle> {
        self.style.as_ref()
    }

    /// The syntax highlighter for fenced code blocks, when set.
    pub(crate) fn code_block_highlighter(&self) -> Option<Arc<CodeBlockHighlighterFn>> {
        self.code_block_highlighter.clone()
    }

    /// Sets the syntax highlighter used for fenced code blocks.
    pub fn with_code_block_highlighter<F>(mut self, highlighter: F) -> Self
    where
        F: Fn(&CodeBlock) -> Vec<(Range<usize>, gpui::HighlightStyle)> + Send + Sync + 'static,
    {
        self.code_block_highlighter = Some(Arc::new(highlighter));
        self
    }

    /// Sets whether text views without an explicit [`TextView::style`] follow
    /// the text color their container sets, default false.
    ///
    /// The body text takes that color. On a surface inverted from the page,
    /// such as a `primary` fill, links, muted text, code, borders and
    /// selection are derived from it too, and the installed syntax highlighter
    /// is left out because its colors are made for the page. Enable this only
    /// when every window sets a text color at its root, as Component's does.
    pub fn with_inherit_text_color(mut self, inherit: bool) -> Self {
        self.inherit_text_color = inherit;
        self
    }

    /// Installs these defaults for the whole application.
    pub fn install(self, cx: &mut App) {
        cx.set_global(self);
    }

    /// Returns the installed defaults, or the Base ones when none were.
    pub fn global(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Whether a syntax highlighter was installed.
    pub fn has_code_block_highlighter(&self) -> bool {
        self.code_block_highlighter.is_some()
    }

    /// Whether text views follow the text color their container sets.
    pub fn inherit_text_color(&self) -> bool {
        self.inherit_text_color
    }
}

/// Type for the table actions generator function.
pub(crate) type TableActionsFn =
    dyn Fn(&TableData, &mut Window, &mut App) -> AnyElement + Send + Sync;

pub(crate) type ImageSourceFn = dyn Fn(&gpui::SharedUri) -> gpui::ImageSource + Send + Sync;

pub(crate) type LinkFn = dyn Fn(&Link, &App) -> Option<Link> + Send + Sync;

/// A link a [`TextView`] draws, as handed to [`TextView::link`].
///
/// Reference links (`[text][id]`) arrive resolved to their definition. New
/// fields may be added later, so build one with [`Link::new`] or change a
/// received one with the setters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Link {
    /// Where the link points; this is what a click opens or hands to
    /// [`TextView::on_link_click`].
    pub url: SharedString,
    /// The Markdown link title, `[text](url "title")`.
    pub title: Option<SharedString>,
}

impl Link {
    /// A link to `url` with no title.
    pub fn new(url: impl Into<SharedString>) -> Self {
        Self {
            url: url.into(),
            title: None,
        }
    }

    /// Point the link at `url`.
    pub fn url(mut self, url: impl Into<SharedString>) -> Self {
        self.url = url.into();
        self
    }

    /// Set the link title.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
}

pub(crate) type LinkClickHandlerFn =
    dyn Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync;

pub(crate) type LinkTooltipFn =
    dyn Fn(&SharedString, &mut Window, &mut App) -> AnyView + Send + Sync;
pub(crate) type TaskToggleHandlerFn = dyn Fn(&TaskToggleEvent, &mut Window, &mut App) + Send + Sync;

/// A click on the checkbox of a Markdown task list item, passed to the
/// [`TextView::on_task_toggle`] handler.
///
/// The ranges are byte ranges in the view's source text.
#[derive(Debug, Clone)]
pub struct TaskToggleEvent {
    checked: bool,
    item_range: Range<usize>,
    marker_range: Range<usize>,
}

impl TaskToggleEvent {
    pub(crate) fn new(checked: bool, item_range: Range<usize>, marker_range: Range<usize>) -> Self {
        Self {
            checked,
            item_range,
            marker_range,
        }
    }

    /// The checked state the item should now have: the opposite of the
    /// state its checkbox shows.
    pub fn checked(&self) -> bool {
        self.checked
    }

    /// The list item's source span, starting at its list marker (`-`, `*`,
    /// `1.`) and covering its content, nested lists included.
    pub fn item_range(&self) -> Range<usize> {
        self.item_range.clone()
    }

    /// The item's task marker: the `[ ]` or `[x]` itself, brackets included.
    pub fn marker_range(&self) -> Range<usize> {
        self.marker_range.clone()
    }
}

/// Kept by the element only, so unlike the handlers the state carries, it
/// may hold a `ScrollHandle`.
pub(crate) type RevealHandlerFn = dyn Fn(Bounds<Pixels>, &mut Window, &mut App);

pub(crate) fn handle_link_click(
    handler: &Option<Arc<LinkClickHandlerFn>>,
    url: SharedString,
    event: ClickEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(handler) = handler {
        handler(&url, &event, window, cx);
    } else if match &event {
        ClickEvent::Mouse(click) => {
            matches!(click.up.button, MouseButton::Left | MouseButton::Middle)
        }
        ClickEvent::Keyboard(_) => true,
        ClickEvent::Touch(click) => !click.long_press,
    } {
        cx.open_url(&url);
    }
}

/// A text view that can render Markdown or HTML.
///
/// ## Goals
///
/// - Provide a rich text rendering component for such as Markdown or HTML,
/// used to display rich text in GPUI application (e.g., Help messages, Release notes)
/// - Support Markdown GFM and HTML (Simple HTML like Safari Reader Mode) for showing most common used markups.
/// - Support Heading, Paragraph, Bold, Italic, StrikeThrough, Code, Link, Image, Blockquote, List, Table, HorizontalRule, CodeBlock ...
///
/// ## Not Goals
///
/// - Customization of the complex style (some simple styles will be supported)
/// - As a Markdown editor or viewer (If you want to like this, you must fork your version).
/// - As a HTML viewer, we not support CSS, we only support basic HTML tags for used to as a content reader.
///
/// See also [`MarkdownElement`], [`HtmlElement`]
#[derive(Clone)]
pub struct TextView {
    id: ElementId,
    format: Option<TextViewFormat>,
    text: Option<SharedString>,
    pub(crate) state: Option<Entity<TextViewState>>,
    text_view_style: Option<TextViewStyle>,
    inline_code_style: Option<StyleRefinement>,
    list_style: Option<StyleRefinement>,
    blockquote_style: Option<StyleRefinement>,
    style: StyleRefinement,
    selectable: bool,
    selection_format: SelectionFormat,
    scrollable: bool,
    max_lines: Option<usize>,
    code_block_actions: Option<Arc<CodeBlockActionsFn>>,
    code_block_highlighter: Option<Arc<CodeBlockHighlighterFn>>,
    table_actions: Option<Arc<TableActionsFn>>,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    link_hook: Option<Arc<LinkFn>>,
    link_tooltip: Option<Arc<LinkTooltipFn>>,
    task_toggle_handler: Option<Arc<TaskToggleHandlerFn>>,
    image_source: Option<Arc<ImageSourceFn>>,
    reveal_handler: Option<Rc<RevealHandlerFn>>,
    markdown_extensions: Arc<MarkdownExtensions>,
    motion: Option<TextViewMotion>,
}

/// A plugin that can configure a [`TextView`].
pub trait TextViewPlugin {
    fn setup(self, text_view: TextView) -> TextView;
}

impl<P> TextViewPlugin for P
where
    P: MarkdownPlugin,
{
    fn setup(self, mut text_view: TextView) -> TextView {
        let extensions = Arc::make_mut(&mut text_view.markdown_extensions);
        let current = std::mem::take(extensions);
        *extensions = current.plugin(self);
        text_view
    }
}

impl Styled for TextView {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl TextView {
    /// Create new TextView with managed state.
    pub fn new(state: &Entity<TextViewState>) -> Self {
        Self {
            id: ElementId::Name(state.entity_id().to_string().into()),
            state: Some(state.clone()),
            format: None,
            text: None,
            text_view_style: None,
            inline_code_style: None,
            list_style: None,
            blockquote_style: None,
            style: StyleRefinement::default(),
            selectable: true,
            selection_format: SelectionFormat::default(),
            scrollable: false,
            max_lines: None,
            code_block_actions: None,
            code_block_highlighter: None,
            table_actions: None,
            link_click_handler: None,
            link_hook: None,
            link_tooltip: None,
            task_toggle_handler: None,
            image_source: None,
            reveal_handler: None,
            markdown_extensions: Arc::default(),
            motion: None,
        }
    }

    /// Create a new markdown text view.
    pub fn markdown(id: impl Into<ElementId>, markdown: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            format: Some(TextViewFormat::Markdown),
            text: Some(markdown.into()),
            text_view_style: None,
            inline_code_style: None,
            list_style: None,
            blockquote_style: None,
            style: StyleRefinement::default(),
            state: None,
            selectable: true,
            selection_format: SelectionFormat::default(),
            scrollable: false,
            max_lines: None,
            code_block_actions: None,
            code_block_highlighter: None,
            table_actions: None,
            link_click_handler: None,
            link_hook: None,
            link_tooltip: None,
            task_toggle_handler: None,
            image_source: None,
            reveal_handler: None,
            markdown_extensions: Arc::default(),
            motion: None,
        }
    }

    /// Create a new html text view.
    pub fn html(id: impl Into<ElementId>, html: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            format: Some(TextViewFormat::Html),
            text: Some(html.into()),
            text_view_style: None,
            inline_code_style: None,
            list_style: None,
            blockquote_style: None,
            style: StyleRefinement::default(),
            state: None,
            selectable: true,
            selection_format: SelectionFormat::default(),
            scrollable: false,
            max_lines: None,
            code_block_actions: None,
            code_block_highlighter: None,
            table_actions: None,
            link_click_handler: None,
            link_hook: None,
            link_tooltip: None,
            task_toggle_handler: None,
            image_source: None,
            reveal_handler: None,
            markdown_extensions: Arc::default(),
            motion: None,
        }
    }

    /// Overrides the source of every document image, including embedded data URLs.
    ///
    /// Used for both rendering and intrinsic-size measurement. The returned source
    /// is authoritative: pending or failed loads never fall back to the document URL.
    /// Without this override, images use Base's default URI and data URL handling.
    pub fn image_source<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&gpui::SharedUri) -> gpui::ImageSource + Send + Sync + 'static,
    {
        self.image_source = Some(Arc::new(resolver));
        self
    }

    /// Refines inline code typography, background, border, radius, and padding
    /// over the resolved style. See [`TextViewStyle::with_inline_code_style`]
    /// for the supported properties. Unspecified properties are inherited.
    pub fn inline_code_style(mut self, style: StyleRefinement) -> Self {
        self.inline_code_style = Some(super::style::normalize_inline_code_style(style));
        self
    }

    /// Refines list markers over the resolved style. See
    /// [`TextViewStyle::with_list_style`] for where it applies.
    /// Unspecified properties are inherited.
    pub fn list_style(mut self, style: StyleRefinement) -> Self {
        self.list_style = Some(style);
        self
    }

    /// Refines blockquotes over the resolved style. See
    /// [`TextViewStyle::with_blockquote_style`] for where it applies.
    /// Unspecified properties are inherited.
    pub fn blockquote_style(mut self, style: StyleRefinement) -> Self {
        self.blockquote_style = Some(style);
        self
    }

    /// Set [`TextViewStyle`].
    pub fn style(mut self, style: TextViewStyle) -> Self {
        self.text_view_style = Some(style);
        self
    }

    /// Set whether the text view is selectable, default is true.
    ///
    /// Hovering selectable text shows the I-beam cursor. A cursor set on the
    /// text view itself, such as `.cursor_default()`, replaces it: use that for
    /// text on a surface that opens on click. Links keep the pointing hand, and
    /// the I-beam still shows while a drag selection is in progress.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Set the [`SelectionFormat`], default is [`SelectionFormat::Plain`].
    ///
    /// With [`SelectionFormat::Source`], selecting inside `**bold**` yields
    /// `**bold**` (the Markdown source) rather than `bold`.
    pub fn selection_format(mut self, selection_format: SelectionFormat) -> Self {
        self.selection_format = selection_format;
        self
    }

    /// Set the text view to be scrollable, default is false.
    ///
    /// ## If true for `scrollable`
    ///
    /// The `scrollable` mode used for large content,
    /// will show scrollbar, but requires the parent to have a fixed height,
    /// and use [`gpui::list`] to render the content in a virtualized way.
    ///
    /// ## If false to fit content
    ///
    /// The TextView will expand to fit all content, no scrollbar.
    /// This mode is suitable for small content, such as a few lines of text, a label, etc.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Clamp the rendered content to at most `n` lines of body text.
    ///
    /// The view's height is capped at `n` × the base line height, and a line
    /// of glyphs is never cut in half: a line that would straddle the bottom
    /// of the box is left out whole, across paragraphs, lists, headings, code
    /// blocks and tables. Nothing is shown with less than a line of itself to
    /// show, so the border and padding a table row leads with never strands at
    /// the bottom; whatever has more than that is cut on the box edge and keeps
    /// the part that fits, rather than disappearing and leaving blank space
    /// behind.
    ///
    /// Check [`TextViewState::is_clamped`] (which answers for the frame that
    /// was last painted) to decide whether to show an "expand" affordance.
    ///
    /// `n` counts lines of body text, so paragraph spacing and taller lines
    /// mean fewer of them fit inside the capped height. A line taller than the
    /// whole budget keeps the part that fits rather than emptying the box.
    /// Ignored when [`Self::scrollable`] is set.
    pub fn max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = Some(max_lines);
        self
    }

    /// Set custom block actions for code blocks.
    ///
    /// The closure receives the [`CodeBlock`],
    /// and returns an element to display.
    pub fn code_block_actions<F, E>(mut self, f: F) -> Self
    where
        F: Fn(&CodeBlock, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.code_block_actions = Some(Arc::new(move |code_block, window, cx| {
            f(&code_block, window, cx).into_any_element()
        }));
        self
    }

    /// Adds opt-in syntax highlighting for fenced code blocks.
    ///
    /// Returned byte ranges are relative to [`CodeBlock::code`]. Invalid
    /// ranges are discarded. Without this callback, code is unhighlighted.
    pub fn code_block_highlighter<F>(mut self, highlighter: F) -> Self
    where
        F: Fn(&CodeBlock) -> Vec<(Range<usize>, gpui::HighlightStyle)> + Send + Sync + 'static,
    {
        self.code_block_highlighter = Some(Arc::new(highlighter));
        self
    }

    /// Like [`Self::code_block_highlighter`], with a highlighter that can be
    /// handed to every frame.
    ///
    /// A code block reuses its highlights only while the highlighter is the
    /// same `Arc`, so a view built every frame with a fresh closure
    /// re-highlights every code block on every frame.
    #[doc(hidden)]
    pub fn shared_code_block_highlighter(
        mut self,
        highlighter: Arc<
            dyn Fn(&CodeBlock) -> Vec<(Range<usize>, gpui::HighlightStyle)> + Send + Sync,
        >,
    ) -> Self {
        self.code_block_highlighter = Some(highlighter);
        self
    }

    /// Set custom actions to be rendered below each Markdown table.
    ///
    /// The closure receives the [`TableData`],
    /// and returns an element to display.
    pub fn table_actions<F, E>(mut self, f: F) -> Self
    where
        F: Fn(&TableData, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.table_actions = Some(Arc::new(move |table, window, cx| {
            f(table, window, cx).into_any_element()
        }));
        self
    }

    /// Handle pointer events on rendered links.
    ///
    /// The handler receives the resolved URL and the original GPUI click event.
    /// Without a handler, links open through App::open_url.
    pub fn on_link_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.link_click_handler = Some(Arc::new(handler));
        self
    }

    /// Replace each link before it is drawn.
    ///
    /// `f` receives every link, inline links, reference links (resolved to
    /// their definition) and linked images alike, and returns the link to
    /// draw in its place: the same link, one with a different URL or title,
    /// or `None` to draw the text like the text around it, with no link
    /// color or underline, and not clickable.
    ///
    /// `f` runs each time the text is drawn, so what it returns may follow
    /// application state (a record that was deleted, a URL resolved later);
    /// keep it cheap, with no I/O or locks held across frames.
    ///
    /// ```ignore
    /// TextView::markdown(id, text).link(|link, cx| {
    ///     let record = parse_record_url(&link.url)?;
    ///     records(cx).contains(record).then(|| link.clone().url(record.web_url()))
    /// })
    /// ```
    pub fn link<F>(mut self, f: F) -> Self
    where
        F: Fn(&Link, &App) -> Option<Link> + Send + Sync + 'static,
    {
        self.link_hook = Some(Arc::new(f));
        self
    }

    /// Show a tooltip that follows the pointer while it is over a link.
    ///
    /// The builder receives the link's URL. The view caches the tooltip
    /// until a different destination is shown.
    pub fn link_tooltip<F>(mut self, builder: F) -> Self
    where
        F: Fn(&SharedString, &mut Window, &mut App) -> AnyView + Send + Sync + 'static,
    {
        self.link_tooltip = Some(Arc::new(builder));
        self
    }

    /// Make the checkboxes of Markdown task list items (`- [ ]`, `- [x]`)
    /// clickable.
    ///
    /// The handler receives a [`TaskToggleEvent`] with the checked state the
    /// item should now have and where the item and its `[ ]` / `[x]` marker
    /// are in the source text. The view does not change its text: the caller
    /// owns the source, replaces the marker and sets the new text. Without a
    /// handler, the checkboxes are not interactive.
    pub fn on_task_toggle<F>(mut self, handler: F) -> Self
    where
        F: Fn(&TaskToggleEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.task_toggle_handler = Some(Arc::new(handler));
        self
    }

    /// Scroll a container that does not follow scroll requests to the line
    /// of [`TextViewState::reveal_range`].
    ///
    /// A `gpui::list` scrolls to that line by itself; a `div` with
    /// `overflow_y_scroll`, for one, does not. After a frame in which the
    /// line was laid out but not visible, the handler receives its bounds in
    /// window coordinates, to scroll the container, e.g. through its
    /// `ScrollHandle`, until the line is visible.
    pub fn on_reveal<F>(mut self, handler: F) -> Self
    where
        F: Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    {
        self.reveal_handler = Some(Rc::new(handler));
        self
    }

    /// Replace the Markdown extension registry.
    pub fn markdown_extensions(mut self, extensions: MarkdownExtensions) -> Self {
        self.markdown_extensions = Arc::new(extensions);
        self
    }

    /// Set the motion policy; see [`TextViewMotion`]. Without one, the
    /// state's own policy applies, which plays no motion by default.
    pub fn motion(mut self, motion: TextViewMotion) -> Self {
        self.motion = Some(motion);
        self
    }

    /// Enable MDX JSX/expression parsing.
    ///
    /// This disables raw HTML parsing because `markdown-rs` gives HTML
    /// priority over MDX when both are enabled.
    pub fn markdown_mdx(mut self) -> Self {
        let extensions = Arc::make_mut(&mut self.markdown_extensions);
        *extensions = extensions.clone().mdx();
        self
    }

    /// Set whether raw HTML in Markdown renders as HTML (the default) or
    /// shows as the text it is written as. See [`MarkdownExtensions::raw_html`].
    pub fn markdown_raw_html(mut self, enabled: bool) -> Self {
        let extensions = Arc::make_mut(&mut self.markdown_extensions);
        *extensions = extensions.clone().raw_html(enabled);
        self
    }

    /// Register a custom block-level Markdown parser.
    ///
    /// The parser runs during Markdown AST conversion and must be independent
    /// of [`Window`] / [`App`]. Store any parsed data in [`MarkdownNode`] and
    /// render it later with [`Self::markdown_block_renderer`].
    pub fn markdown_block_parser<F>(mut self, parser: F) -> Self
    where
        F: for<'a> Fn(
                &markdown::mdast::Node,
                &crate::text::MarkdownParseContext<'a>,
            ) -> Option<MarkdownNode>
            + Send
            + Sync
            + 'static,
    {
        Arc::make_mut(&mut self.markdown_extensions).push_block_parser(parser);
        self
    }

    /// Register a renderer for a custom block-level Markdown node name.
    pub fn markdown_block_renderer<F, E>(
        mut self,
        name: impl Into<SharedString>,
        renderer: F,
    ) -> Self
    where
        F: Fn(&MarkdownNode, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        Arc::make_mut(&mut self.markdown_extensions).push_block_renderer(name, renderer);
        self
    }

    /// Apply a reusable text view plugin.
    pub fn plugin<P>(self, plugin: P) -> Self
    where
        P: TextViewPlugin,
    {
        plugin.setup(self)
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl IntoElement for TextView {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct TextViewLayoutState {
    state: Entity<TextViewState>,
    element: AnyElement,
}

pub struct TextViewPrepaintState {
    hitbox: Hitbox,
    /// Where paint has to pull the `max_lines` clip up to, because a glyph line
    /// straddles the bottom of the box. `None` leaves the clip at the box edge,
    /// where the container's hidden overflow already applies it.
    clip_bottom: Option<Pixels>,
    /// The touch handles this view owns, with their hitboxes.
    touch_handles: crate::TouchHandleLayout,
}

/// Absorbs sub-pixel layout jitter: a line ending within a pixel of the box
/// bottom counts as fitting inside it.
const CLIP_EPSILON: Pixels = px(1.);

/// The bottom of the last whole line at or above `y`, with the height of a line
/// where it sits.
fn last_line_bottom_above(spans: &[LineSpan], y: Pixels) -> Option<(Pixels, Pixels)> {
    let mut last: Option<(Pixels, Pixels)> = None;
    let mut keep = |bottom: Pixels, line_height: Pixels| {
        if bottom <= y + CLIP_EPSILON && last.is_none_or(|(last, _)| bottom > last) {
            last = Some((bottom, line_height));
        }
    };

    for span in spans {
        if span.line_height <= px(0.) {
            continue;
        }
        let mut bottom = span.top + span.line_height;
        while bottom <= span.bottom + CLIP_EPSILON {
            keep(bottom, span.line_height);
            bottom += span.line_height;
        }
        // The span's own bottom covers a last line taller than the rest.
        keep(span.bottom, span.line_height);
    }

    last
}

/// Where to clip, given the lines a descendant `Inline` reported. `None` leaves
/// the clip on the box edge.
///
/// Two things are never shown: half a line of glyphs, and anything with less
/// than a line of itself to show. A line straddling `box_bottom` is left out
/// whole, and so is the strip between it and the line before — the border and
/// padding a table row leads with reads as a rendering fault rather than as a
/// row. Whatever has more than a line to show is cut on the edge and keeps the
/// part that fits, so the box holds no blank space it could have filled.
fn line_safe_clip_bottom(
    spans: &[LineSpan],
    box_bottom: Pixels,
    content_bottom: Pixels,
) -> Option<Pixels> {
    let mut clip = box_bottom;

    for span in spans {
        if span.line_height <= px(0.)
            || span.top >= box_bottom
            || span.bottom <= box_bottom + CLIP_EPSILON
        {
            continue;
        }
        let whole_lines = ((box_bottom - span.top) / span.line_height).floor();
        let line_top = span.top + span.line_height * whole_lines;
        // A line starting on the box edge is not straddling it.
        if line_top < box_bottom - CLIP_EPSILON {
            clip = clip.min(line_top);
        }
    }

    let Some((last_line_bottom, line_height)) = last_line_bottom_above(spans, clip) else {
        // Leaving the straddling line out would leave nothing at all — a first
        // line taller than the whole budget, a heading in a one-line box. It
        // keeps the part that fits instead, because an empty clamp reads as
        // broken where a cut one reads as more to come.
        return None;
    };

    // Snap away a scrap. Only content that continues past the box can leave
    // one: the space under the last line of a document that fits is the box's
    // own, not a piece of something below.
    if content_bottom > box_bottom + CLIP_EPSILON {
        let strip = clip - last_line_bottom;
        if strip > CLIP_EPSILON && strip < line_height {
            clip = last_line_bottom;
        }
    }

    (clip < box_bottom - CLIP_EPSILON).then_some(clip)
}

impl Element for TextView {
    type RequestLayoutState = TextViewLayoutState;
    type PrepaintState = TextViewPrepaintState;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = if let Some(state) = self.state.clone() {
            state
        } else {
            let default_format = self.format.unwrap_or(TextViewFormat::Markdown);
            let default_text = self.text.clone().unwrap_or_default();

            let state = window.use_keyed_state(
                SharedString::from(format!("{}/state", self.id)),
                cx,
                move |_, cx| {
                    if default_format == TextViewFormat::Markdown {
                        TextViewState::markdown(default_text.as_str(), cx)
                    } else {
                        TextViewState::html(default_text.as_str(), cx)
                    }
                },
            );
            self.state = Some(state.clone());
            state
        };

        // `max_lines` needs the whole document laid out to snap the clip to a
        // whole line, so it only applies to the fit-content mode.
        let max_lines = self.max_lines.filter(|_| !self.scrollable);

        // Resolve the style by reference: this runs every frame, and the
        // style only reaches the state when it changed.
        let defaults = cx.try_global::<TextViewDefaults>();
        let theme_style;
        let mut text_view_style = match (
            &self.text_view_style,
            defaults.and_then(|d| d.style.as_ref()),
        ) {
            (Some(style), _) | (None, Some(style)) => style,
            (None, None) => {
                theme_style = TextViewStyle::from_theme(&crate::Theme::global(cx));
                &theme_style
            }
        };
        // Follow the text color the container sets for its surface, so rich
        // text stays readable in a filled bubble or a selected row.
        let surface_style;
        let mut on_inverted_surface = false;
        if self.text_view_style.is_none() && defaults.is_some_and(|d| d.inherit_text_color) {
            let color = self
                .style
                .text
                .color
                .unwrap_or_else(|| window.text_style().color);
            if color != text_view_style.foreground() {
                on_inverted_surface = text_view_style.is_inverted_by(color);
                surface_style = text_view_style.on_text_color(color);
                text_view_style = &surface_style;
            }
        }
        let inline_style;
        if let Some(refinement) = &self.inline_code_style {
            let mut style = text_view_style.inline_code_style().clone();
            style.refine(refinement);
            inline_style = text_view_style.clone().with_inline_code_style(style);
            text_view_style = &inline_style;
        }
        let block_style;
        if self.list_style.is_some() || self.blockquote_style.is_some() {
            let mut style = text_view_style.clone();
            if let Some(refinement) = &self.list_style {
                let mut marker = style.list_style().clone();
                marker.refine(refinement);
                style = style.with_list_style(marker);
            }
            if let Some(refinement) = &self.blockquote_style {
                let mut quote = style.blockquote_style().clone();
                quote.refine(refinement);
                style = style.with_blockquote_style(quote);
            }
            block_style = style;
            text_view_style = &block_style;
        }
        let foreground = text_view_style.foreground();
        let text_view_style = (*state.read(cx).text_view_style != *text_view_style)
            .then(|| Arc::new(text_view_style.clone()));
        let code_block_highlighter = self.code_block_highlighter.clone().or_else(|| {
            defaults
                .filter(|_| !on_inverted_surface)
                .and_then(|d| d.code_block_highlighter.clone())
        });

        state.update(cx, |state, cx| {
            if text_view_style.is_some()
                || match (&state.code_block_highlighter, &code_block_highlighter) {
                    (Some(previous), Some(next)) => !Arc::ptr_eq(previous, next),
                    (None, None) => false,
                    _ => true,
                }
            {
                state.preserve_selection_for_reflow();
            }
            state.code_block_actions = self.code_block_actions.clone();
            state.code_block_highlighter = code_block_highlighter;
            state.table_actions = self.table_actions.clone();
            state.link_click_handler = self.link_click_handler.clone();
            state.link_hook = self.link_hook.clone();
            state.link_tooltip = self.link_tooltip.clone();
            state.task_toggle_handler = self.task_toggle_handler.clone();
            state.image_source = self.image_source.clone();
            state.set_markdown_extensions(self.markdown_extensions.clone(), cx);
            if let Some(motion) = &self.motion {
                state.set_motion(motion.clone());
            }
            state.selectable = self.selectable;
            state.hover_cursor = self.style.mouse_cursor;
            state.selection_format = self.selection_format;
            state.scrollable = self.scrollable;
            state.max_lines = max_lines;
            if let Some(text_view_style) = text_view_style {
                state.text_view_style = text_view_style;
            }

            if let Some(text) = &self.text {
                state.set_element_text(text, cx);
            }
        });

        let focus_handle = state.read(cx).focus_handle.clone();
        let list_state = state.read(cx).list_state.clone();
        // Cap the box at `n` body-text lines (the effective text style may be
        // refined by this view's own style, e.g. `.text_sm()`); hidden
        // overflow also clips descendant hitboxes to the box during prepaint.
        let max_lines_cap = max_lines.map(|max_lines| {
            let mut text_style = window.text_style();
            text_style.refine(&self.style.text);
            text_style.line_height_in_pixels(window.rem_size()) * max_lines as f32
        });

        let mut el = div()
            .id(("text-view-scroll", state.entity_id()))
            .key_context("TextView")
            .track_focus(&focus_handle)
            .when(self.scrollable, |this| this.size_full())
            .when_some(max_lines_cap, |this, cap| this.max_h(cap).overflow_hidden())
            .relative()
            // Paragraphs start on the start edge; LTR keeps the inherited
            // alignment, and the view's own style still overrides both.
            .when(crate::LayoutDirection::global(cx).is_rtl(), |this| {
                crate::LogicalStyledExt::text_start(this, cx)
            })
            .text_color(foreground)
            .on_action(move |_: &crate::input::Copy, window, cx| {
                let text = TextSelection::selected_text(window, cx).trim().to_string();
                if text.is_empty() {
                    cx.propagate();
                    return;
                }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            })
            .on_action(window.listener_for(&state, TextViewState::on_action_select_all))
            .map(|this| {
                if self.scrollable {
                    this.child(crate::ScrollBounce::new(
                        ("text-view-bounce", state.entity_id()),
                        &list_state,
                        state.clone(),
                    ))
                } else {
                    this.child(state.clone())
                }
            })
            // Overlay controls must paint after the document, otherwise rich
            // content and selection backgrounds cover the thumb and hitbox.
            .when(self.scrollable, |this| {
                this.child(
                    div().absolute().inset_0().child(
                        crate::Scrollbar::vertical(&list_state)
                            .id(("text-view-scrollbar", state.entity_id()))
                            .viewport_from_layout(),
                    ),
                )
            })
            .refine_style(&self.style)
            .into_any_element();
        let layout_id = el.request_layout(window, cx);
        (layout_id, TextViewLayoutState { state, element: el })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = request_layout.state.clone();
        let max_lines_active = state.read(cx).max_lines.is_some();
        if max_lines_active {
            if let Ok(mut line_spans) = state.read(cx).line_spans.lock() {
                line_spans.clear();
            }
        }
        // Descendants read the view through the state stack during prepaint
        // (in addition to the paint-time push below): `Inline`s report their
        // line spans when `max_lines` is set, and table cells look for the
        // selection anchor.
        GlobalState::global(cx).push_text_view_state(state.clone());
        request_layout.element.prepaint(window, cx);
        GlobalState::global(cx).pop_text_view_state();

        let mut clip_bottom = None;
        if max_lines_active {
            let (line_spans, content_bottom) = {
                let state = state.read(cx);
                (
                    state
                        .line_spans
                        .lock()
                        .map(|spans| spans.clone())
                        .unwrap_or_default(),
                    state.bounds().bottom(),
                )
            };
            // The content keeps its natural height inside the capped box, so
            // this sees everything the box cannot show — including a tall image
            // that reports no lines of its own.
            let clipped = content_bottom > bounds.bottom() + px(1.);
            // Notify on change so observers (e.g. an "expand" button gated on
            // `is_clamped`) re-render once the flag flips.
            if state.read(cx).clamped != clipped {
                state.update(cx, |state, cx| {
                    state.clamped = clipped;
                    cx.notify();
                });
            }
            if clipped {
                clip_bottom = line_safe_clip_bottom(&line_spans, bounds.bottom(), content_bottom);
            }
        }

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        // Over the text, so after its hitbox.
        let touch_handles = if self.selectable {
            state
                .read(cx)
                .selection_adapter
                .prepaint_touch_handles(window, cx)
        } else {
            crate::TouchHandleLayout::default()
        };
        TextViewPrepaintState {
            hitbox,
            clip_bottom,
            touch_handles,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let state = &request_layout.state;
        if self.selectable {
            state.read(cx).selection_adapter.begin_frame();
        }

        GlobalState::global(cx).push_text_view_state(state.clone());
        if let Some(clip_bottom) = prepaint.clip_bottom {
            // Snap the `max_lines` clip to the last whole line that fits, so a
            // line of glyphs is never cut in half.
            let mask = ContentMask {
                bounds: Bounds::from_corners(bounds.origin, point(bounds.right(), clip_bottom)),
            };
            window.with_content_mask(Some(mask), |window| {
                request_layout.element.paint(window, cx);
            });
        } else {
            request_layout.element.paint(window, cx);
        }
        GlobalState::global(cx).pop_text_view_state();

        // Every list has scrolled by now, so the line of a reveal is where
        // it ends up this frame.
        if state.read(cx).pending_reveal.is_some() {
            let progress = state.update(cx, |state, _| {
                state.pending_reveal.as_mut().map(PendingReveal::progress)
            });
            match progress {
                Some(RevealProgress::Shown) => {
                    state.update(cx, |state, _| state.pending_reveal = None);
                }
                Some(RevealProgress::Hidden(line)) => {
                    if let Some(handler) = &self.reveal_handler {
                        handler(line, window, cx);
                    }
                }
                Some(RevealProgress::NotLaidOut) | None => {}
            }
        }

        if self.selectable {
            let (adapter, scroll_offset, content_bounds, self_scroll, handle_color) = {
                let state = state.read(cx);
                (
                    state.selection_adapter.clone(),
                    state.scroll_offset(),
                    state.bounds(),
                    state.scrollable,
                    state.text_view_style.selection().alpha(1.),
                )
            };
            adapter.register(
                prepaint.hitbox.clone(),
                content_bounds,
                scroll_offset,
                self_scroll,
                window,
                cx,
            );
            // The handles of a touch selection go over the text, and under
            // whatever is painted over the text after it.
            adapter.paint_touch_handles(&prepaint.touch_handles, handle_color, window, cx);
        }
    }
}
