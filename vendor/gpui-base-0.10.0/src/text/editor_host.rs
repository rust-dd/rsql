//! Host API for external editors.
//!
//! An editor that lives outside gpui-kit keeps its own document model and
//! hands gpui-kit a [`BlockNode`] tree, the same tree [`TextView`] renders.
//! gpui-kit renders it and gives back, per text leaf, the geometry the editor
//! needs for hit-testing, caret placement and platform input (IME) bounds.
//!
//! The boundary is deliberately narrow:
//!
//! - **In:** a [`BlockNode`] tree whose text leaves carry a host-chosen `u64`
//!   key (see [`Paragraph::with_leaf_key`] and [`CodeBlock::with_leaf_key`]),
//!   a [`RenderContext`], and per-key decoration ranges.
//! - **Out:** one opaque [`LeafGeometry`] per key from
//!   [`collect_leaf_geometry`].
//!
//! Nothing lower is exposed: no `TextLayout`, no layout internals, no sink
//! trait, no retained-layout or projection caches.
//!
//! The host owns editing semantics, document revisions, accessibility and native
//! input integration. This module provides horizontal TextView block rendering;
//! it is not a complete visual-caret navigation or pagination API. Extend the
//! public boundary additively while keeping rendering internals private.
//!
//! [`TextView`]: super::TextView

use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, Bounds, ClickEvent, ElementId, HighlightStyle, Hsla, ImageSource,
    InteractiveElement as _, IntoElement as _, ParentElement as _, Pixels, Point, SharedString,
    SharedUri, TextLayout, Window, WrappedLineLayout, div, point, px,
};
use unicode_segmentation::UnicodeSegmentation as _;

use super::{
    TextViewDefaults, TextViewStyle,
    document::{NodeRenderOptions, PrevBlock, flow_position},
    editor_seam::{self, EditorSink},
    format::markdown,
    markdown_ext::MarkdownExtensions,
    node::NodeContext,
};

pub use super::node::{
    BlockNode, BlockState, CodeBlock, ColumnumnAlign as ColumnAlign, ImageNode, InlineNode,
    LinkMark, Paragraph, Span, Table, TextMark,
};

/// Parses Markdown with the parser [`TextView`](super::TextView) uses,
/// including the given extensions, into the block tree.
pub fn parse_markdown(
    source: &str,
    extensions: Arc<MarkdownExtensions>,
) -> Result<Vec<BlockNode>, SharedString> {
    let mut cx = NodeContext {
        markdown_extensions: extensions,
        ..Default::default()
    };
    let parsed = markdown::parse(source, &mut cx)?;
    Ok(Arc::unwrap_or_clone(parsed.blocks))
}

/// What [`BlockNode::render`] needs besides the node: the text style, code
/// highlighter and Markdown extensions.
#[derive(Clone)]
pub struct RenderContext {
    node: NodeContext,
}

impl RenderContext {
    /// Creates a local rendering context without reading application globals.
    /// The host supplies body typography through its container or
    /// `with_body_font_size`.
    pub fn new(style: TextViewStyle) -> Self {
        Self {
            node: NodeContext {
                style: Arc::new(style),
                ..Default::default()
            },
        }
    }

    /// Replaces code highlighting for this context only.
    pub fn with_code_block_highlighter<F>(mut self, highlighter: F) -> Self
    where
        F: Fn(&CodeBlock) -> Vec<(Range<usize>, HighlightStyle)> + Send + Sync + 'static,
    {
        self.node.code_block_highlighter = Some(Arc::new(highlighter));
        self
    }

    /// Resolves document image URIs. The host owns resource access policy.
    pub fn with_image_source<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&SharedUri) -> ImageSource + Send + Sync + 'static,
    {
        self.node.image_source = Some(Arc::new(resolver));
        self
    }

    /// Handles link activation. Without a handler, links use App::open_url.
    pub fn on_link_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.node.link_click_handler = Some(Arc::new(handler));
        self
    }

    /// Registers a reference-link definition from the host's document.
    /// Supply definitions for the entire document, including virtualized blocks.
    pub fn with_link_reference(
        mut self,
        identifier: impl Into<SharedString>,
        link: LinkMark,
    ) -> Self {
        self.node.link_refs.insert(identifier.into(), link);
        self
    }

    /// The context TextView would start from: the global [`TextViewDefaults`]
    /// style and code-block highlighter when set, else the theme's style.
    pub fn from_app(cx: &App) -> Self {
        let theme = crate::Theme::global(cx);
        let defaults = cx.try_global::<TextViewDefaults>();
        let style = defaults
            .and_then(|defaults| defaults.style().cloned())
            .unwrap_or_else(|| TextViewStyle::from_theme(&theme));
        Self {
            node: NodeContext {
                style: Arc::new(style),
                code_block_highlighter: defaults.and_then(|d| d.code_block_highlighter()),
                ..Default::default()
            },
        }
    }

    /// Replaces the text style.
    pub fn with_style(mut self, style: TextViewStyle) -> Self {
        self.node.style = Arc::new(style);
        self
    }

    /// Supplies this render's foreground ranges for a keyed paragraph leaf,
    /// including a heading or table cell. Replaces any ranges for the same key.
    ///
    /// Ranges use the leaf's rendered UTF-8 bytes; images and custom inline
    /// objects occupy one U+FFFC (three bytes), as in [`LeafGeometry`]. Text
    /// ranges are clipped to each text fragment; non-character boundaries and
    /// empty or reversed ranges are ignored. An object's foreground changes
    /// only when a range covers its entire placeholder; images are unchanged.
    ///
    /// These colors override mark and link foregrounds; later ranges win
    /// overlaps. Other mark properties and link activation are preserved.
    /// Uncovered text inherits its ordinary foreground. Colors belong to this
    /// rendering context, never the document tree: supply current theme/surface
    /// colors on each render, or omit the ranges to restore inheritance.
    pub fn with_foreground_ranges(mut self, key: u64, ranges: Vec<(Range<usize>, Hsla)>) -> Self {
        let foregrounds = self.node.foregrounds.get_or_insert_with(Default::default);
        Arc::make_mut(foregrounds).insert(key, ranges);
        self
    }

    /// Sets the Markdown extensions that render custom nodes.
    pub fn with_markdown_extensions(mut self, extensions: Arc<MarkdownExtensions>) -> Self {
        self.node.markdown_extensions = extensions;
        self
    }

    /// Sets the font size body text inherits, which the block gaps and
    /// heading sizes are relative to.
    pub fn with_body_font_size(mut self, size: Pixels) -> Self {
        self.node.body_font_size = size;
        self
    }

    /// The text style blocks render with.
    pub fn style(&self) -> &TextViewStyle {
        &self.node.style
    }
}

/// Where a block stands in the flow it is rendered in, which decides the gaps
/// around it: the first block takes no gap above, the last none below, and a
/// heading or rule counts the block before it.
#[derive(Clone, Copy, Debug)]
pub struct FlowPosition {
    is_first: bool,
    is_last: bool,
    prev: PrevBlock,
}

impl FlowPosition {
    /// The position of `blocks[ix]` among its siblings. Blocks that render
    /// nothing (definitions, breaks) are skipped when looking for the block
    /// before it.
    pub fn at(blocks: &[&BlockNode], ix: usize, is_last: bool) -> Self {
        let (is_first, prev) = flow_position(blocks, ix);
        Self {
            is_first,
            is_last,
            prev,
        }
    }

    /// Overrides whether the block opens its flow, e.g. for a chunk of a
    /// long list rendered as a list of its own.
    pub fn first(mut self, is_first: bool) -> Self {
        self.is_first = is_first;
        self
    }

    /// Overrides whether the block closes its flow.
    pub fn last(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

impl BlockNode {
    /// Renders under a stable host-owned identity. Use a domain ID, not a list
    /// position, when inserting, reordering or virtualizing blocks. Identity
    /// scopes GPUI element state independently of the leaf keys used for geometry.
    pub fn render_with_id(
        &self,
        id: impl Into<ElementId>,
        flow: FlowPosition,
        render_context: &RenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        div()
            .id(id)
            .child(self.render(0, flow, render_context, window, cx))
            .into_any_element()
    }

    /// Renders the block as TextView does. `ix` identifies the block among
    /// its siblings (element ids derive from it).
    pub fn render(
        &self,
        ix: usize,
        flow: FlowPosition,
        cx: &RenderContext,
        window: &mut Window,
        app: &mut App,
    ) -> AnyElement {
        self.render_block(
            NodeRenderOptions {
                ix,
                is_first: flow.is_first,
                is_last: flow.is_last,
                prev: flow.prev,
                ..Default::default()
            },
            &cx.node,
            window,
            app,
        )
    }
}

/// Runs `f`, which should prepaint rendered blocks, and returns its result
/// with the geometry every keyed text leaf reported meanwhile.
///
/// `decorations` maps a leaf key to byte ranges of that leaf's text (atoms
/// count as one `U+FFFC` each) to paint behind the text: the host's selection,
/// find matches, anything. Leaves without a key, or without an entry, paint as
/// usual. Atom decorations cover the whole atom and paint over its content
/// so selections remain visible over opaque images. Supply translucent colors
/// for overlays. Ranges are half-open; empty ranges paint nothing.
/// Scopes nest, and the previous one is restored even if `f` panics.
///
/// Keys must be unique within one collection scope. Nested scopes are isolated:
/// an outer scope does not collect the inner scope's leaves. Only prepainted
/// leaves are returned; absence is not evidence that a document leaf was removed.
/// The host owns document revisions, virtualization, focus, native input and
/// UTF-16/UTF-8 conversion for platform input APIs.
pub fn collect_leaf_geometry<R>(
    decorations: HashMap<u64, Vec<(Range<usize>, Hsla)>>,
    f: impl FnOnce() -> R,
) -> (R, HashMap<u64, LeafGeometry>) {
    let collector = Rc::new(Collector {
        pieces: RefCell::default(),
        decorations,
    });
    let result = editor_seam::with_sink(collector.clone(), f);
    let mut pieces = collector.pieces.take();
    pieces.retain(|_, pieces| !pieces.is_empty());
    let leaves = pieces
        .into_iter()
        .map(|(key, pieces)| (key, LeafGeometry { pieces }))
        .collect();
    (result, leaves)
}

struct Collector {
    pieces: RefCell<HashMap<u64, Vec<Piece>>>,
    decorations: HashMap<u64, Vec<(Range<usize>, Hsla)>>,
}

impl EditorSink for Collector {
    fn report(&self, key: u64, layout: &TextLayout) {
        self.pieces
            .borrow_mut()
            .insert(key, vec![Piece::Whole(LayoutSnapshot::new(layout))]);
    }

    fn begin_pieces(&self, key: u64) {
        self.pieces.borrow_mut().insert(key, Vec::new());
    }

    fn report_text_piece(&self, key: u64, start: usize, layout: &TextLayout) {
        self.pieces
            .borrow_mut()
            .entry(key)
            .or_default()
            .push(Piece::Text {
                start,
                layout: LayoutSnapshot::new(layout),
            });
    }

    fn report_box_piece(&self, key: u64, range: Range<usize>, bounds: Bounds<Pixels>) {
        self.pieces
            .borrow_mut()
            .entry(key)
            .or_default()
            .push(Piece::Atom { range, bounds });
    }

    fn report_empty_piece(&self, key: u64, start: usize, bounds: Bounds<Pixels>) {
        self.pieces
            .borrow_mut()
            .entry(key)
            .or_default()
            .push(Piece::Empty { start, bounds });
    }

    fn backgrounds(&self, key: u64) -> Vec<(Range<usize>, Hsla)> {
        self.decorations.get(&key).cloned().unwrap_or_default()
    }
}

/// Copy mutable frame state, sharing only immutable shaped lines.
#[derive(Clone)]
struct LayoutSnapshot {
    bounds: Bounds<Pixels>,
    len: usize,
    line_height: Pixels,
    text: SharedString,
    lines: Vec<Arc<WrappedLineLayout>>,
}

impl LayoutSnapshot {
    fn new(layout: &TextLayout) -> Self {
        Self {
            bounds: layout.bounds(),
            len: layout.len(),
            line_height: layout.line_height(),
            text: layout.text().into(),
            lines: layout.line_layouts().into_iter().collect(),
        }
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
    fn len(&self) -> usize {
        self.len
    }
    fn line_height(&self) -> Pixels {
        self.line_height
    }
    fn text(&self) -> &str {
        &self.text
    }
    fn line_layouts(&self) -> &[Arc<WrappedLineLayout>] {
        &self.lines
    }

    fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        let mut origin = self.bounds.origin;
        let mut start = 0;
        for line in &self.lines {
            if index <= start + line.len() {
                return line
                    .position_for_index(index - start, self.line_height)
                    .map(|p| origin + p);
            }
            start += line.len() + 1;
            origin.y += line.size(self.line_height).height;
        }
        None
    }
}

/// How a leaf was laid out: one layout for the whole leaf, or (for a mixed
/// paragraph rendered as a flow) single-line text fragments and atom boxes in
/// leaf byte order.
#[derive(Clone)]
enum Piece {
    Whole(LayoutSnapshot),
    Text {
        start: usize,
        layout: LayoutSnapshot,
    },
    Empty {
        start: usize,
        bounds: Bounds<Pixels>,
    },
    Atom {
        range: Range<usize>,
        bounds: Bounds<Pixels>,
    },
}

impl Piece {
    fn bounds(&self) -> Bounds<Pixels> {
        match self {
            Piece::Whole(layout) | Piece::Text { layout, .. } => layout.bounds(),
            Piece::Atom { bounds, .. } | Piece::Empty { bounds, .. } => *bounds,
        }
    }

    fn range(&self) -> Range<usize> {
        match self {
            Piece::Whole(layout) => 0..layout.len(),
            Piece::Text { start, layout } => *start..start + layout.len(),
            Piece::Atom { range, .. } => range.clone(),
            Piece::Empty { start, .. } => *start..*start,
        }
    }
}

/// Which side of a soft wrap a caret sits on.
///
/// At a soft wrap one byte offset names two caret positions: the end of the
/// upper visual line and the start of the lower one. Same terms as AppKit's
/// `NSSelectionAffinity` and Flutter's `TextAffinity`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    /// The end of the visual line before the offset.
    Upstream,
    /// The start of the visual line after the offset.
    #[default]
    Downstream,
}

/// A caret position: a byte offset plus the side of a soft wrap it is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Caret {
    /// The byte offset into the leaf's text.
    pub index: usize,
    /// The visual line the caret is on when `index` is at a soft wrap.
    pub affinity: CaretAffinity,
}

impl Caret {
    /// A caret at `index`, at the start of a line at a soft wrap.
    pub fn new(index: usize) -> Self {
        Self {
            index,
            affinity: CaretAffinity::Downstream,
        }
    }

    /// A caret at `index`, at the end of a line at a soft wrap.
    pub fn upstream(index: usize) -> Self {
        Self {
            index,
            affinity: CaretAffinity::Upstream,
        }
    }
}

/// Where one keyed text leaf was laid out in the last frame.
///
/// Indices are byte offsets into the leaf's text, where each inline atom
/// (image, custom node) is one `U+FFFC` (3 bytes). Coordinates are window
/// pixels and are valid for the frame that produced them. It is a value, not a
/// handle: it owns its bounds and shares immutable shaped lines, never the
/// renderer's mutable layout state. Later frames do not mutate this snapshot.
/// The host must replace it after content, width, typography or scrolling changes.
#[derive(Clone)]
pub struct LeafGeometry {
    pieces: Vec<Piece>,
}

impl LeafGeometry {
    fn whole(&self) -> Option<&LayoutSnapshot> {
        match self.pieces.as_slice() {
            [Piece::Whole(layout)] => Some(layout),
            _ => None,
        }
    }

    /// The bounds of the leaf: the union of its pieces.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.pieces
            .iter()
            .map(Piece::bounds)
            .reduce(|a, b| a.union(&b))
            .unwrap_or_default()
    }

    /// The length of the leaf's text in bytes, atoms included.
    pub fn len(&self) -> usize {
        self.pieces
            .iter()
            .map(|piece| piece.range().end)
            .max()
            .unwrap_or(0)
    }

    /// Whether the leaf's text is empty (it still has a line box).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The height of a line.
    pub fn line_height(&self) -> Pixels {
        self.pieces
            .iter()
            .find_map(|piece| match piece {
                Piece::Whole(layout) | Piece::Text { layout, .. } => Some(layout.line_height()),
                Piece::Empty { bounds, .. } => Some(bounds.size.height),
                Piece::Atom { .. } => None,
            })
            .unwrap_or_else(|| self.bounds().size.height)
    }

    /// The height of a caret: the font's ascent plus descent, not the whole
    /// line box, which includes leading.
    pub fn caret_height(&self) -> Pixels {
        let line_height = self.line_height();
        self.pieces
            .iter()
            .find_map(|piece| match piece {
                Piece::Whole(layout) | Piece::Text { layout, .. } => {
                    let line = layout.lines.first()?;
                    let h = line.unwrapped_layout.ascent + line.unwrapped_layout.descent;
                    (h > Pixels::ZERO).then_some(h)
                }
                Piece::Atom { .. } | Piece::Empty { .. } => None,
            })
            .unwrap_or(line_height * 0.85)
            .min(line_height)
    }

    /// The top of a caret placed at byte `index`, in window coordinates.
    pub fn position_for_index(&self, index: usize) -> Point<Pixels> {
        if let Some(layout) = self.whole() {
            return layout
                .position_for_index(index.min(layout.len()))
                .unwrap_or_else(|| self.bounds().origin);
        }
        // The piece the index starts, else the one it ends (a caret at a
        // fragment boundary sits at the start of the next fragment).
        let piece = self
            .pieces
            .iter()
            .find(|p| p.range().contains(&index))
            .or_else(|| self.pieces.iter().rev().find(|p| p.range().end == index))
            .or(self.pieces.last());
        match piece {
            Some(piece) => self.position_in_piece(piece, index),
            None => self.bounds().origin,
        }
    }

    /// The byte index of the caret position closest to `at`: glyph halves
    /// round to the nearer side, unlike `TextLayout::index_for_position`.
    pub fn index_for_position(&self, at: Point<Pixels>) -> usize {
        if let Some(layout) = self.whole() {
            return whole_index_at(layout, at);
        }
        let Some(piece) = self.hit_piece(at) else {
            return 0;
        };
        let index = match piece {
            Piece::Whole(layout) => whole_index_at(layout, at),
            // Fragments are one line: take the caret position nearest to the
            // point (`closest_index_for_position` rounds up at an exact
            // boundary inside scaled code text).
            // Only grapheme boundaries are candidates: an interior byte of a
            // multi-byte character maps to the next glyph's x, so it could
            // tie with (and beat) the real boundary and split the character.
            Piece::Text { start, layout } => {
                let text = layout.text();
                let nearest = text
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain(std::iter::once(text.len()))
                    .filter_map(|i| {
                        layout
                            .position_for_index(i)
                            .map(|p| (i, (p.x - at.x).abs()))
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map_or(0, |(i, _)| i);
                start + nearest
            }
            Piece::Empty { start, .. } => *start,
            Piece::Atom { range, bounds } => {
                if at.x < bounds.center().x {
                    range.start
                } else {
                    range.end
                }
            }
        };
        index.min(self.len())
    }

    /// The caret closest to `at`, keeping the visual line that was hit: at a
    /// soft wrap the end of the upper line is [`CaretAffinity::Upstream`].
    /// Pass it to [`Self::position_for_caret`] to round-trip a click.
    pub fn caret_for_position(&self, at: Point<Pixels>) -> Caret {
        if let Some(layout) = self.whole() {
            return whole_caret_at(layout, at);
        }
        let index = self.index_for_position(at);
        // The hit ends a piece (the right end of a line in the flow): stay on
        // the line that was hit, not the next piece's.
        let ends_hit_line = self.hit_piece(at).is_some_and(|piece| {
            let range = piece.range();
            range.start < range.end && range.end == index
        });
        if ends_hit_line {
            Caret::upstream(index)
        } else {
            Caret::new(index)
        }
    }

    /// The top of `caret` in window coordinates. Unlike
    /// [`Self::position_for_index`], a caret at a soft wrap lands on the
    /// visual line its affinity names.
    pub fn position_for_caret(&self, caret: Caret) -> Point<Pixels> {
        let index = caret.index;
        if let Some(layout) = self.whole() {
            return whole_position_for_caret(layout, caret).unwrap_or_else(|| self.bounds().origin);
        }
        match caret.affinity {
            CaretAffinity::Downstream => {
                // A caret at the start of a piece belongs to that piece.
                let starts = self
                    .pieces
                    .iter()
                    .find(|p| p.range().start == index && p.range().end > index);
                match starts {
                    Some(piece) => self.position_in_piece(piece, index),
                    None => self.position_for_index(index),
                }
            }
            CaretAffinity::Upstream => {
                let ends = self
                    .pieces
                    .iter()
                    .find(|p| p.range().start < index && p.range().end == index);
                match ends {
                    Some(piece) => self.position_in_piece(piece, index),
                    None => self.position_for_index(index),
                }
            }
        }
    }

    fn position_in_piece(&self, piece: &Piece, index: usize) -> Point<Pixels> {
        match piece {
            Piece::Whole(layout) | Piece::Text { layout, .. } => {
                let local = index.saturating_sub(piece.range().start).min(layout.len());
                layout
                    .position_for_index(local)
                    .unwrap_or_else(|| layout.bounds().origin)
            }
            Piece::Empty { bounds, .. } => bounds.origin,
            Piece::Atom { range, bounds } => {
                let x = if index >= range.end {
                    bounds.right()
                } else {
                    bounds.left()
                };
                let y = (bounds.bottom() - self.line_height()).max(bounds.top());
                point(x, y)
            }
        }
    }

    /// The piece under `at`: the visual line under the point (pieces whose
    /// vertical extent holds it, else the nearest), then the horizontally
    /// nearest piece.
    fn hit_piece(&self, at: Point<Pixels>) -> Option<&Piece> {
        let dy = |p: &Piece| {
            let b = p.bounds();
            if at.y < b.top() {
                b.top() - at.y
            } else if at.y > b.bottom() {
                at.y - b.bottom()
            } else {
                px(0.)
            }
        };
        let dx = |p: &Piece| {
            let b = p.bounds();
            if at.x < b.left() {
                b.left() - at.x
            } else if at.x > b.right() {
                at.x - b.right()
            } else {
                px(0.)
            }
        };
        self.pieces.iter().min_by(|a, b| {
            (dy(a), dx(a))
                .partial_cmp(&(dy(b), dx(b)))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// The byte range of the inline atom drawn under `at`, if any.
    pub fn atom_at(&self, at: Point<Pixels>) -> Option<Range<usize>> {
        self.pieces.iter().find_map(|piece| match piece {
            Piece::Atom { range, bounds } if bounds.contains(&at) => Some(range.clone()),
            _ => None,
        })
    }
}

/// The closest index in a whole-leaf layout for a window point.
fn whole_index_at(layout: &LayoutSnapshot, at: Point<Pixels>) -> usize {
    let bounds = layout.bounds();
    let line_height = layout.line_height();
    let mut top = bounds.top();
    let mut start = 0;
    let lines = layout.line_layouts();
    for (ix, line) in lines.iter().enumerate() {
        let height = line.size(line_height).height.max(line_height);
        let is_last = ix + 1 == lines.len();
        if at.y < top + height || is_last {
            let local = point(
                (at.x - bounds.left()).max(px(0.)),
                (at.y - top).clamp(px(0.), (height - px(1.)).max(px(0.))),
            );
            let index = match line.closest_index_for_position(local, line_height) {
                Ok(ix) | Err(ix) => ix,
            };
            return start + index;
        }
        top += height;
        start += line.len() + 1;
    }
    0
}

/// The byte offsets where each soft-wrapped row after the first starts.
fn wrap_starts(line: &WrappedLineLayout) -> impl Iterator<Item = usize> + '_ {
    line.wrap_boundaries
        .iter()
        .map(|b| line.unwrapped_layout.runs[b.run_ix].glyphs[b.glyph_ix].index)
}

/// [`whole_index_at`] plus the affinity of the visual row that was hit.
fn whole_caret_at(layout: &LayoutSnapshot, at: Point<Pixels>) -> Caret {
    let index = whole_index_at(layout, at);
    let bounds = layout.bounds();
    let line_height = layout.line_height();
    let mut top = bounds.top();
    let mut start = 0;
    let lines = layout.line_layouts();
    for (ix, line) in lines.iter().enumerate() {
        let height = line.size(line_height).height.max(line_height);
        if at.y < top + height || ix + 1 == lines.len() {
            let row = ((at.y - top).max(px(0.)) / line_height) as usize;
            // The hit row ends where the next row starts: an index there is
            // the end of the hit row.
            let row_end = wrap_starts(line).nth(row).map(|i| start + i);
            return if row_end == Some(index) {
                Caret::upstream(index)
            } else {
                Caret::new(index)
            };
        }
        top += height;
        start += line.len() + 1;
    }
    Caret::new(index)
}

/// The top of `caret` in a whole-leaf layout, on the row its affinity names.
fn whole_position_for_caret(layout: &LayoutSnapshot, caret: Caret) -> Option<Point<Pixels>> {
    let index = caret.index.min(layout.len());
    let line_height = layout.line_height();
    let mut origin = layout.bounds().origin;
    let mut start = 0;
    for line in layout.line_layouts() {
        if index <= start + line.len() {
            let local = index - start;
            let unwrapped = &line.unwrapped_layout;
            // Row starts: 0, then each wrap. Pick the last row starting at or
            // before `local`; upstream steps back off a row that starts there.
            let mut row = 0;
            let mut row_start = 0;
            for (ix, wrap) in wrap_starts(line).enumerate() {
                if wrap < local || (wrap == local && caret.affinity == CaretAffinity::Downstream) {
                    row = ix + 1;
                    row_start = wrap;
                } else {
                    break;
                }
            }
            let x = unwrapped.x_for_index(local) - unwrapped.x_for_index(row_start);
            return Some(origin + point(x, line_height * row as f32));
        }
        start += line.len() + 1;
        origin.y += line.size(line_height).height;
    }
    None
}
