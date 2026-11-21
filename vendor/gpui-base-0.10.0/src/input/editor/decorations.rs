use crate::input::{EditorMode, TextareaExtras, TextareaMode};
use std::{collections::BTreeMap, ops::Range};

use gpui::{App, Context, HighlightStyle, Hsla, Pixels, WeakEntity};
use ropey::Rope;
use sum_tree::Bias;

use super::{InputBaseState, RopeExt as _};

/// Geometric presentation for an editor range decoration.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RangeDecorationStyle {
    /// Fill the continuous visual range.
    Fill,
    /// Draw a continuous one-pixel frame around the visual range.
    #[default]
    Frame,
}

/// A geometric decoration over a UTF-8 byte range.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeDecoration {
    range: Range<usize>,
    style: RangeDecorationStyle,
    color: Option<Hsla>,
}

impl RangeDecoration {
    /// Create a frame using the editor foreground color.
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            style: RangeDecorationStyle::default(),
            color: None,
        }
    }

    /// The half-open UTF-8 byte range supplied to this decoration.
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// The geometric paint style.
    pub fn style(&self) -> RangeDecorationStyle {
        self.style
    }

    /// An application-owned color override, or `None` for the editor fallback.
    pub fn color(&self) -> Option<Hsla> {
        self.color
    }

    /// Choose a fill or frame without changing text layout.
    pub fn with_style(mut self, style: RangeDecorationStyle) -> Self {
        self.style = style;
        self
    }

    /// Override the editor foreground fallback with an application-owned color.
    pub fn with_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

/// A mark that the editor draws in a strip of the gutter, before the line numbers.
///
/// A non-empty range marks every line it touches. This matches Monaco's
/// [`linesDecorationsClassName`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IModelDecorationOptions.html#linesDecorationsClassName)
/// with `isWholeLine`. An empty range marks a line boundary, such as where lines
/// were deleted. When the range is at the start of a line, the mark appears above
/// that line. Otherwise it appears below the line.
///
/// The default is an empty range at the start of the document, a boundary above
/// the first line, in the editor foreground color.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GutterDecoration {
    range: Range<usize>,
    color: Option<Hsla>,
    folded_color: Option<Hsla>,
}

impl GutterDecoration {
    /// Create a mark with the editor foreground color.
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            color: None,
            folded_color: None,
        }
    }

    /// The decoration's half-open range of UTF-8 byte offsets.
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// The application color override, or `None` to use the editor fallback.
    pub fn color(&self) -> Option<Hsla> {
        self.color
    }

    /// The color for the bar when the range starts inside a fold, or `None` to
    /// use [`Self::color`].
    pub fn folded_color(&self) -> Option<Hsla> {
        self.folded_color
    }

    /// Set the color instead of the editor foreground fallback.
    pub fn with_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Set the color for the bar on a fold's row when the range starts inside that fold.
    pub fn with_folded_color(mut self, color: Hsla) -> Self {
        self.folded_color = Some(color);
        self
    }
}

/// The lane that an [`OverviewRulerDecoration`] uses on the vertical scrollbar track.
///
/// This matches Monaco's
/// [`OverviewRulerLane`](https://microsoft.github.io/monaco-editor/typedoc/enums/editor_editor_api.editor.OverviewRulerLane.html).
/// Its convention uses `Left` for version control, `Center` for search results,
/// and `Right` for diagnostics.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverviewRulerLane {
    /// The left third of the track.
    Left,
    /// The middle third of the track.
    #[default]
    Center,
    /// The right third of the track.
    Right,
    /// The whole width of the track.
    Full,
}

/// A mark that the editor draws on the vertical scrollbar track for a UTF-8 byte range.
///
/// This is the counterpart of Monaco's
/// [`overviewRuler`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IModelDecorationOverviewRulerOptions.html)
/// decoration option. Like a [`GutterDecoration`], an empty range marks a line boundary.
///
/// The default is an empty range at the start of the document in the center lane,
/// in the editor foreground color.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverviewRulerDecoration {
    range: Range<usize>,
    lane: OverviewRulerLane,
    color: Option<Hsla>,
}

impl OverviewRulerDecoration {
    /// Create a mark in the center lane with the editor foreground color.
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            lane: OverviewRulerLane::default(),
            color: None,
        }
    }

    /// The decoration's half-open range of UTF-8 byte offsets.
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// The lane that this mark uses on the track.
    pub fn lane(&self) -> OverviewRulerLane {
        self.lane
    }

    /// The application color override, or `None` to use the editor fallback.
    pub fn color(&self) -> Option<Hsla> {
        self.color
    }

    /// Set the lane that this mark uses.
    pub fn with_lane(mut self, lane: OverviewRulerLane) -> Self {
        self.lane = lane;
        self
    }

    /// Set the color instead of the editor foreground fallback.
    pub fn with_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

/// A presentation style applied to a UTF-8 byte range in an input.
///
/// This is the GPUI [`HighlightStyle`] counterpart of Monaco's
/// [`IModelDeltaDecoration`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IModelDeltaDecoration.html).
#[derive(Clone, Debug, PartialEq)]
pub struct TextDecoration {
    pub range: Range<usize>,
    pub style: HighlightStyle,
}

impl TextDecoration {
    /// Create a text decoration from a UTF-8 byte range and a GPUI style.
    pub fn new(range: Range<usize>, style: HighlightStyle) -> Self {
        Self { range, style }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct DecorationCollectionId(usize);

/// An independently managed collection of [`TextDecoration`]s.
///
/// This is the GPUI Component counterpart of Monaco's
/// [`IEditorDecorationsCollection`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html).
#[derive(Clone, Debug)]
pub struct TextDecorationCollection {
    state: WeakEntity<InputBaseState<EditorMode>>,
    id: DecorationCollectionId,
}

impl TextDecorationCollection {
    /// Replace all decorations in this collection.
    ///
    /// This corresponds to Monaco's
    /// [`IEditorDecorationsCollection.set`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html#set).
    pub fn set(&self, decorations: Vec<TextDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state.extras.decorations.set(self.id, decorations) {
                cx.notify();
            }
        });
    }

    /// Add decorations to this collection.
    ///
    /// This corresponds to Monaco's
    /// [`IEditorDecorationsCollection.append`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html#append).
    pub fn append(&self, decorations: Vec<TextDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state.extras.decorations.append(self.id, decorations) {
                cx.notify();
            }
        });
    }

    /// Remove all decorations from this collection.
    ///
    /// This corresponds to Monaco's
    /// [`IEditorDecorationsCollection.clear`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html#clear).
    pub fn clear(&self, cx: &mut App) {
        self.set(Vec::new(), cx);
    }

    /// Return the UTF-8 byte ranges in this collection.
    ///
    /// This corresponds to Monaco's
    /// [`IEditorDecorationsCollection.getRanges`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html#getRanges).
    pub fn get_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.state
            .read_with(cx, |state, _| {
                state
                    .extras
                    .decorations
                    .get(self.id)
                    .unwrap_or_default()
                    .iter()
                    .map(|decoration| decoration.range.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// An independently managed collection of geometric range decorations.
///
/// Clones address the same collection. Dropping a handle does not clear it; use
/// [`Self::clear`] to empty it or [`Self::dispose`] to release it permanently.
/// Operations on a disposed collection or a dropped editor are harmless no-ops.
#[derive(Clone, Debug)]
pub struct RangeDecorationCollection {
    state: WeakEntity<InputBaseState<EditorMode>>,
    id: DecorationCollectionId,
}

impl RangeDecorationCollection {
    /// Replace only this owner's decorations, clipping ranges to UTF-8 boundaries.
    pub fn set(&self, decorations: Vec<RangeDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state.extras.range_decorations.set(self.id, decorations) {
                cx.notify();
            }
        });
    }

    /// Append decorations, preserving their paint order.
    pub fn append(&self, decorations: Vec<RangeDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state.extras.range_decorations.append(self.id, decorations) {
                cx.notify();
            }
        });
    }

    /// Empty this collection without invalidating its handles.
    pub fn clear(&self, cx: &mut App) {
        self.set(Vec::new(), cx);
    }

    /// Release this collection, invalidating all of its cloned handles.
    pub fn dispose(&self, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            if state
                .extras
                .range_decorations
                .entries
                .remove(&self.id)
                .is_some()
            {
                cx.notify();
            }
        });
    }

    /// Read tracked UTF-8 byte ranges in insertion order.
    pub fn get_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.state
            .read_with(cx, |state, _| {
                state
                    .extras
                    .range_decorations
                    .get(self.id)
                    .unwrap_or_default()
                    .iter()
                    .map(|decoration| decoration.range.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// An independently managed collection of [`GutterDecoration`] values.
///
/// Each clone refers to the same collection. Dropping a handle does not clear it.
/// Call [`Self::clear`] to empty it or [`Self::dispose`] to release it. Calls after
/// disposal or after the editor is dropped do nothing.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct GutterDecorationCollection {
    state: WeakEntity<InputBaseState<EditorMode>>,
    id: DecorationCollectionId,
}

/// A handle that addresses no editor, so every operation on it does nothing. It
/// can stand in for a collection until the editor that owns one exists.
impl Default for GutterDecorationCollection {
    fn default() -> Self {
        Self {
            state: WeakEntity::new_invalid(),
            id: DecorationCollectionId(0),
        }
    }
}

impl GutterDecorationCollection {
    /// Replace this collection's decorations and clip ranges to UTF-8 boundaries.
    pub fn set(&self, decorations: Vec<GutterDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state.extras.gutter_decorations.set(self.id, decorations) {
                cx.notify();
            }
        });
    }

    /// Add decorations to this collection in paint order.
    pub fn append(&self, decorations: Vec<GutterDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state.extras.gutter_decorations.append(self.id, decorations) {
                cx.notify();
            }
        });
    }

    /// Empty this collection and keep its handles usable. The gutter keeps its strip.
    pub fn clear(&self, cx: &mut App) {
        self.set(Vec::new(), cx);
    }

    /// Release this collection. Its cloned handles stop working. The gutter drops
    /// its strip after the last gutter collection is released.
    pub fn dispose(&self, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            if state.extras.gutter_decorations.remove(self.id) {
                cx.notify();
            }
        });
    }

    /// Return tracked UTF-8 byte ranges in insertion order.
    pub fn get_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.state
            .read_with(cx, |state, _| {
                state
                    .extras
                    .gutter_decorations
                    .get(self.id)
                    .unwrap_or_default()
                    .iter()
                    .map(|decoration| decoration.range.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// An independently managed collection of [`OverviewRulerDecoration`] values.
///
/// Each clone refers to the same collection. Dropping a handle does not clear it.
/// Call [`Self::clear`] to empty it or [`Self::dispose`] to release it. Calls after
/// disposal or after the editor is dropped do nothing.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct OverviewRulerDecorationCollection {
    state: WeakEntity<InputBaseState<EditorMode>>,
    id: DecorationCollectionId,
}

/// A handle that addresses no editor, so every operation on it does nothing. It
/// can stand in for a collection until the editor that owns one exists.
impl Default for OverviewRulerDecorationCollection {
    fn default() -> Self {
        Self {
            state: WeakEntity::new_invalid(),
            id: DecorationCollectionId(0),
        }
    }
}

impl OverviewRulerDecorationCollection {
    /// Replace this collection's decorations and clip ranges to UTF-8 boundaries.
    pub fn set(&self, decorations: Vec<OverviewRulerDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state
                .extras
                .overview_ruler_decorations
                .set(self.id, decorations)
            {
                cx.notify();
            }
        });
    }

    /// Add decorations to this collection in paint order.
    pub fn append(&self, decorations: Vec<OverviewRulerDecoration>, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            let decorations = normalize(&state.text, decorations);
            if state
                .extras
                .overview_ruler_decorations
                .append(self.id, decorations)
            {
                cx.notify();
            }
        });
    }

    /// Empty this collection and keep its handles usable.
    pub fn clear(&self, cx: &mut App) {
        self.set(Vec::new(), cx);
    }

    /// Release this collection. Its cloned handles stop working.
    pub fn dispose(&self, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            if state.extras.overview_ruler_decorations.remove(self.id) {
                cx.notify();
            }
        });
    }

    /// Return tracked UTF-8 byte ranges in insertion order.
    pub fn get_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.state
            .read_with(cx, |state, _| {
                state
                    .extras
                    .overview_ruler_decorations
                    .get(self.id)
                    .unwrap_or_default()
                    .iter()
                    .map(|decoration| decoration.range.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Both text styles and geometric decorations share normalization and edit affinity.
pub(crate) trait TrackedDecoration {
    /// Whether an empty range is a decoration of its own, a line boundary, rather
    /// than nothing. Such a range survives the edits that empty it and stays before
    /// text inserted at its offset.
    const KEEPS_EMPTY: bool = false;

    fn range(&self) -> &Range<usize>;
    fn range_mut(&mut self) -> &mut Range<usize>;
}

impl TrackedDecoration for TextDecoration {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

impl TrackedDecoration for RangeDecoration {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

impl TrackedDecoration for GutterDecoration {
    const KEEPS_EMPTY: bool = true;

    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

impl TrackedDecoration for OverviewRulerDecoration {
    const KEEPS_EMPTY: bool = true;

    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

/// A balanced interval index over stable insertion-order entries. Each midpoint
/// stores the maximum end of its subtree, so one document-spanning decoration
/// does not force a scan of every preceding decoration on each frame.
struct DecorationIndex {
    indices: Vec<usize>,
    max_ends: Vec<usize>,
}

impl DecorationIndex {
    fn new<T: TrackedDecoration>(decorations: &[T]) -> Self {
        let mut indices: Vec<_> = (0..decorations.len()).collect();
        indices.sort_unstable_by_key(|&ix| (decorations[ix].range().start, ix));
        let mut index = Self {
            max_ends: vec![0; indices.len()],
            indices,
        };
        index.build(decorations, 0..decorations.len());
        index
    }

    fn build<T: TrackedDecoration>(&mut self, decorations: &[T], span: Range<usize>) -> usize {
        if span.is_empty() {
            return 0;
        }
        let mid = span.start + span.len() / 2;
        let end = decorations[self.indices[mid]]
            .range()
            .end
            .max(self.build(decorations, span.start..mid))
            .max(self.build(decorations, mid + 1..span.end));
        self.max_ends[mid] = end;
        end
    }

    // Returns the number of visited nodes, allowing deterministic complexity tests.
    fn query<T: TrackedDecoration>(
        &self,
        decorations: &[T],
        span: Range<usize>,
        range: &Range<usize>,
        matches: &mut Vec<usize>,
    ) -> usize {
        if span.is_empty() || range.is_empty() {
            return 0;
        }
        // An empty decoration that is kept, a line boundary, touches the range
        // when it lies inside it, including at its start.
        let touches = |end: usize, empty: bool| {
            end > range.start || (T::KEEPS_EMPTY && empty && end == range.start)
        };
        let mid = span.start + span.len() / 2;
        if !touches(self.max_ends[mid], true) {
            return 1;
        }
        let mut visited = 1 + self.query(decorations, span.start..mid, range, matches);
        let ix = self.indices[mid];
        let candidate = decorations[ix].range();
        if candidate.start < range.end {
            if touches(candidate.end, candidate.is_empty()) {
                matches.push(ix);
            }
            visited += self.query(decorations, mid + 1..span.end, range, matches);
        }
        visited
    }
}

struct DecorationEntries<T> {
    decorations: Vec<T>,
    index: DecorationIndex,
}

impl<T: TrackedDecoration> DecorationEntries<T> {
    fn new(decorations: Vec<T>) -> Self {
        let index = DecorationIndex::new(&decorations);
        Self { decorations, index }
    }

    fn reindex(&mut self) {
        self.index = DecorationIndex::new(&self.decorations);
    }
}

pub(crate) struct DecorationCollections<T = TextDecoration> {
    entries: BTreeMap<DecorationCollectionId, DecorationEntries<T>>,
    next_id: usize,
    /// Changes whenever an owner creates, replaces, extends, clears or releases a
    /// collection, so painting can keep work derived from them until then.
    revision: u64,
}

impl<T> Default for DecorationCollections<T> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            next_id: 0,
            revision: 0,
        }
    }
}

impl<T: TrackedDecoration> DecorationCollections<T> {
    fn create(&mut self, decorations: Vec<T>) -> DecorationCollectionId {
        let id = DecorationCollectionId(self.next_id);
        self.next_id += 1;
        self.entries.insert(id, DecorationEntries::new(decorations));
        self.revision += 1;
        id
    }

    fn set(&mut self, id: DecorationCollectionId, decorations: Vec<T>) -> bool {
        let Some(current) = self.entries.get_mut(&id) else {
            return false;
        };
        *current = DecorationEntries::new(decorations);
        self.revision += 1;
        true
    }

    fn append(&mut self, id: DecorationCollectionId, decorations: Vec<T>) -> bool {
        let Some(current) = self.entries.get_mut(&id) else {
            return false;
        };
        current.decorations.extend(decorations);
        current.reindex();
        self.revision += 1;
        true
    }

    fn remove(&mut self, id: DecorationCollectionId) -> bool {
        let removed = self.entries.remove(&id).is_some();
        self.revision += u64::from(removed);
        removed
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    fn get(&self, id: DecorationCollectionId) -> Option<&[T]> {
        self.entries
            .get(&id)
            .map(|entry| entry.decorations.as_slice())
    }

    pub(super) fn adjust_for_edit(&mut self, edited_range: &Range<usize>, inserted_len: usize) {
        for entry in self.entries.values_mut() {
            let len = entry.decorations.len();
            if len == 0 || entry.index.max_ends[len / 2] <= edited_range.start {
                continue;
            }
            let mut remap = Vec::with_capacity(len);
            let mut retained = 0;
            entry.decorations.retain_mut(|decoration| {
                let range = decoration.range();
                *decoration.range_mut() = if T::KEEPS_EMPTY && range.is_empty() {
                    let offset = adjust_point_for_edit(range.start, edited_range, inserted_len);
                    offset..offset
                } else {
                    adjust_range_for_edit(range, edited_range, inserted_len)
                };
                let keep = T::KEEPS_EMPTY || !decoration.range().is_empty();
                remap.push(if keep { retained } else { usize::MAX });
                retained += usize::from(keep);
                keep
            });
            // Remap removed entries and restore start ordering when empty and
            // non-empty ranges at the same offset move with different affinity.
            entry.index.indices.retain_mut(|ix| {
                *ix = remap[*ix];
                *ix != usize::MAX
            });
            if !entry
                .index
                .indices
                .is_sorted_by_key(|&ix| (entry.decorations[ix].range().start, ix))
            {
                entry
                    .index
                    .indices
                    .sort_unstable_by_key(|&ix| (entry.decorations[ix].range().start, ix));
            }
            entry.index.max_ends.resize(retained, 0);
            entry.index.build(&entry.decorations, 0..retained);
        }
    }

    pub(super) fn clear(&mut self) {
        for entry in self.entries.values_mut() {
            *entry = DecorationEntries::new(Vec::new());
        }
        self.revision += 1;
    }

    /// Whether any collection exists, even an empty one.
    pub(super) fn has_collections(&self) -> bool {
        !self.entries.is_empty()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &[T]> {
        self.entries
            .values()
            .map(|entry| entry.decorations.as_slice())
    }

    /// Query visible buffer spans (not the intervening folded-away text).
    /// Rebuilds happen on mutations, never in layout/paint. Preserve owner/item
    /// order after deduplicating ranges crossing multiple visible lines.
    pub(super) fn intersecting(&self, ranges: &[Range<usize>]) -> Vec<&T> {
        let mut result = Vec::new();
        for entry in self.entries.values() {
            let mut matches = Vec::new();
            for range in ranges {
                entry.index.query(
                    &entry.decorations,
                    0..entry.decorations.len(),
                    range,
                    &mut matches,
                );
            }
            matches.sort_unstable();
            matches.dedup();
            result.extend(matches.into_iter().map(|ix| &entry.decorations[ix]));
        }
        result
    }
}

fn adjust_range_for_edit(
    range: &Range<usize>,
    edited_range: &Range<usize>,
    inserted_len: usize,
) -> Range<usize> {
    let removed_len = edited_range.end.saturating_sub(edited_range.start);
    let shift = |offset: usize| {
        if inserted_len >= removed_len {
            offset.saturating_add(inserted_len - removed_len)
        } else {
            offset.saturating_sub(removed_len - inserted_len)
        }
    };

    if edited_range.is_empty() {
        let start = if range.start < edited_range.start {
            range.start
        } else {
            shift(range.start)
        };
        let end = if range.end <= edited_range.start {
            range.end
        } else {
            shift(range.end)
        };
        return start..end;
    }

    let inserted_end = edited_range.start + inserted_len;
    let start = if range.start <= edited_range.start {
        range.start
    } else if range.start >= edited_range.end {
        shift(range.start)
    } else {
        edited_range.start
    };
    let end = if range.end <= edited_range.start {
        range.end
    } else if range.end >= edited_range.end {
        shift(range.end)
    } else {
        inserted_end
    };
    start..end
}

/// A point stays before text inserted at its offset, and a replacement that covers
/// it moves it to where the replacement starts.
fn adjust_point_for_edit(offset: usize, edited_range: &Range<usize>, inserted_len: usize) -> usize {
    if offset <= edited_range.start {
        offset
    } else if offset >= edited_range.end {
        (offset + inserted_len).saturating_sub(edited_range.len())
    } else {
        edited_range.start
    }
}

fn normalize<T: TrackedDecoration>(text: &Rope, decorations: Vec<T>) -> Vec<T> {
    decorations
        .into_iter()
        .filter_map(|mut decoration| {
            let range = decoration.range();
            // Reject reversed ranges before clipping, which could otherwise turn a
            // reversed pair within a multibyte character into a nonempty range, and
            // ranges past the end, which clipping would otherwise turn into a
            // boundary at the end of the document.
            if range.start > range.end
                || range.start > text.len()
                || (range.is_empty() && !T::KEEPS_EMPTY)
            {
                return None;
            }
            let start = text.clip_offset(range.start, Bias::Left);
            let range = if range.is_empty() {
                start..start
            } else {
                start..text.clip_offset(range.end, Bias::Right)
            };
            if range.is_empty() && !T::KEEPS_EMPTY {
                return None;
            }
            *decoration.range_mut() = range;
            Some(decoration)
        })
        .collect()
}

impl TextareaExtras {
    pub(crate) fn adjust_for_edit(&mut self, edited_range: &Range<usize>, inserted_len: usize) {
        self.decorations.retain_mut(|decoration| {
            decoration.range = adjust_range_for_edit(&decoration.range, edited_range, inserted_len);
            !decoration.range.is_empty()
        });
    }
}

impl InputBaseState<TextareaMode> {
    /// Replace the textarea's text decorations, as UTF-8 byte ranges into
    /// [`Self::value`]. Ranges follow later edits like an editor's decoration
    /// collections, and one whose text is deleted is dropped.
    pub fn set_decorations(&mut self, decorations: Vec<TextDecoration>, cx: &mut Context<Self>) {
        let decorations = normalize(&self.text, decorations);
        if self.extras.decorations != decorations {
            self.extras.decorations = decorations;
            cx.notify();
        }
    }
    /// The textarea's text decorations, as last set and moved by edits since.
    pub fn decorations(&self) -> &[TextDecoration] {
        &self.extras.decorations
    }
}

impl InputBaseState<EditorMode> {
    /// Create an independently owned collection of geometric range decorations.
    ///
    /// Ranges use UTF-8 byte offsets and the same tracking as text decorations:
    /// insertion at either edge does not expand the range, insertion inside does,
    /// replacement clips overlapping anchors, and deletion removes empty ranges.
    /// Undo, redo, whole-document replacement and formatting apply these same edit
    /// transforms; decorations themselves are not undo history, so deleted ranges
    /// are not resurrected by undo. Folding changes projection, not stored ranges.
    ///
    /// Fills paint behind frames; within each style, later collections/items paint
    /// over earlier ones. Neither affects text layout, hit testing or focus. The
    /// default color is the editor foreground (12% opacity for fills).
    /// Collections live until explicitly disposed or the editor is dropped.
    pub fn create_range_decorations_collection(
        &mut self,
        decorations: Vec<RangeDecoration>,
        cx: &mut Context<Self>,
    ) -> RangeDecorationCollection {
        let id = self
            .extras
            .range_decorations
            .create(normalize(&self.text, decorations));
        cx.notify();
        RangeDecorationCollection {
            state: cx.entity().downgrade(),
            id,
        }
    }

    /// Create an independently managed collection of text decorations.
    ///
    /// This follows Monaco's
    /// [`createDecorationsCollection`](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.ICodeEditor.html#createDecorationsCollection)
    /// ownership model. Ranges use UTF-8 byte offsets into [`Self::value`].
    ///
    /// Decoration ranges follow text edits and do not need to be set again
    /// after each change. Insertions at a range boundary do not expand the
    /// range, matching Monaco's
    /// [`NeverGrowsWhenTypingAtEdges`](https://microsoft.github.io/monaco-editor/typedoc/enums/editor_editor_api.editor.TrackedRangeStickiness.html#NeverGrowsWhenTypingAtEdges)
    /// behavior. Decorations are not rendered while the input is masked.
    /// Collections live until their [`InputBaseState`] is dropped.
    ///
    /// Collections are layered in insertion order; the first collection wins
    /// when overlapping decorations set the same [`HighlightStyle`] property.
    /// Callers should avoid conflicting overlaps within one collection.
    pub fn create_decorations_collection(
        &mut self,
        decorations: Vec<TextDecoration>,
        cx: &mut Context<Self>,
    ) -> TextDecorationCollection {
        let decorations = normalize(&self.text, decorations);
        let id = self.extras.decorations.create(decorations);
        cx.notify();
        TextDecorationCollection {
            state: cx.entity().downgrade(),
            id,
        }
    }

    /// Create a collection of gutter marks, such as added, modified, and deleted
    /// lines in a version-control diff.
    ///
    /// Marks draw in a strip before the line numbers. The strip stays reserved while
    /// any gutter collection exists, including an empty collection, so the text does
    /// not shift when marks change. Release every collection to drop the strip. Its
    /// width is [`Self::gutter_decorations_width`].
    ///
    /// Ranges track edits as [range decorations](Self::create_range_decorations_collection)
    /// do, but empty ranges are kept. They stay before text inserted at their offset,
    /// and a range that an edit empties becomes a boundary. A mark whose range starts
    /// inside a fold draws a bar on the fold's row, using its
    /// [folded color](GutterDecoration::with_folded_color), and marks the lines of the
    /// range after the fold as usual. Later collections and
    /// items draw over earlier ones. After you recompute the decorations, call
    /// [`set`](GutterDecorationCollection::set) with the whole collection. Tracking
    /// keeps marks on their lines between updates.
    pub fn create_gutter_decorations_collection(
        &mut self,
        decorations: Vec<GutterDecoration>,
        cx: &mut Context<Self>,
    ) -> GutterDecorationCollection {
        let id = self
            .extras
            .gutter_decorations
            .create(normalize(&self.text, decorations));
        cx.notify();
        GutterDecorationCollection {
            state: cx.entity().downgrade(),
            id,
        }
    }

    /// Create a collection of marks on the vertical scrollbar track, Monaco's
    /// overview ruler.
    ///
    /// Each mark uses its [lane](OverviewRulerLane) at the rows' share of the content
    /// height. It is at least 6 px tall. Touching marks with the same color merge.
    /// Marks draw under Base's scrollbar or one supplied with
    /// [`Self::set_scrollbar_renderer`], so they remain visible while the thumb fades.
    /// Ranges track edits as [gutter decorations](Self::create_gutter_decorations_collection)
    /// do.
    pub fn create_overview_ruler_decorations_collection(
        &mut self,
        decorations: Vec<OverviewRulerDecoration>,
        cx: &mut Context<Self>,
    ) -> OverviewRulerDecorationCollection {
        let id = self
            .extras
            .overview_ruler_decorations
            .create(normalize(&self.text, decorations));
        cx.notify();
        OverviewRulerDecorationCollection {
            state: cx.entity().downgrade(),
            id,
        }
    }

    /// Set the width of the strip that [gutter decorations](Self::create_gutter_decorations_collection)
    /// draw in. The default is 3 px.
    pub fn gutter_decorations_width(mut self, width: Pixels) -> Self {
        self.extras.gutter_decorations_width = width;
        self
    }

    /// Set the width of the strip that gutter decorations draw in.
    pub fn set_gutter_decorations_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        if self.extras.gutter_decorations_width != width {
            self.extras.gutter_decorations_width = width;
            cx.notify();
        }
    }
}
