use std::{ops::Range, sync::Arc};

use gpui::SharedString;

use super::conflict::DiffConflict;
use super::parser::{DiffParseError, DiffParseOptions};
use smallvec::{SmallVec, smallvec};
use unicode_segmentation::UnicodeSegmentation as _;

/// Which source version a position belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffSide {
    Original,
    Modified,
}

impl DiffSide {
    pub(crate) fn other(self) -> Self {
        match self {
            Self::Original => Self::Modified,
            Self::Modified => Self::Original,
        }
    }
}

/// A one-based source line, independent of the visual layout.
///
/// The file is identified by its [`DiffFile::path`], which stays stable when a
/// newer revision of the patch reorders its files.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffLinePosition {
    path: SharedString,
    side: DiffSide,
    line: usize,
}

impl DiffLinePosition {
    /// Creates a position, clamping the line to at least one.
    pub fn new(path: impl Into<SharedString>, side: DiffSide, line: usize) -> Self {
        Self {
            path: path.into(),
            side,
            line: line.max(1),
        }
    }
    /// The [`DiffFile::path`] of the file.
    pub fn path(&self) -> &SharedString {
        &self.path
    }
    pub fn side(&self) -> DiffSide {
        self.side
    }
    pub fn line(&self) -> usize {
        self.line
    }
}

/// An inclusive range of source lines in one file, identified by its
/// [`DiffFile::path`].
///
/// A range starts at `start` on [`Self::side`] and ends at `end` on
/// [`Self::end_side`]. When both sides are the same it covers that side's lines
/// between the two numbers, in either order. A range ending on the other side
/// covers the patch body between the two lines in patch order, as a Unified
/// view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLineRange {
    path: SharedString,
    side: DiffSide,
    start: usize,
    end_side: DiffSide,
    end: usize,
}

impl DiffLineRange {
    /// Creates a range on one side, clamping the lines to at least one.
    pub fn new(path: impl Into<SharedString>, side: DiffSide, start: usize, end: usize) -> Self {
        Self {
            path: path.into(),
            side,
            start: start.max(1),
            end_side: side,
            end: end.max(1),
        }
    }
    /// Ends the range on `side`, keeping `end` as its line on that side.
    pub fn with_end_side(mut self, side: DiffSide) -> Self {
        self.end_side = side;
        self
    }
    /// The [`DiffFile::path`] of the file.
    pub fn path(&self) -> &SharedString {
        &self.path
    }
    /// The side `start` is on.
    pub fn side(&self) -> DiffSide {
        self.side
    }
    pub fn start(&self) -> usize {
        self.start
    }
    /// The side `end` is on; the same as [`Self::side`] unless set.
    pub fn end_side(&self) -> DiffSide {
        self.end_side
    }
    pub fn end(&self) -> usize {
        self.end
    }
    pub(crate) fn start_position(&self) -> DiffLinePosition {
        DiffLinePosition::new(self.path.clone(), self.side, self.start)
    }
    pub(crate) fn end_position(&self) -> DiffLinePosition {
        DiffLinePosition::new(self.path.clone(), self.end_side, self.end)
    }
    /// Whether the range covers lines on one side only.
    pub(crate) fn is_single_side(&self) -> bool {
        self.side == self.end_side
    }
}

/// One supplied source line and its display form.
#[derive(Debug)]
pub(crate) struct SourceLine {
    line_number: usize,
    text: SharedString,
    display: SharedString,
    chunks: SmallVec<[DisplayChunk; 1]>,
    counterpart: Option<usize>,
    // Only expanded tabs need an entry; ordinary UTF-8 bytes map by identity
    // plus the cumulative expansion of earlier tabs.
    display_tabs: Vec<DisplayTab>,
    source: Range<usize>,
    content_end: usize,
}

#[derive(Debug)]
struct DisplayChunk {
    range: Range<usize>,
    text: SharedString,
}

#[derive(Debug)]
struct DisplayTab {
    source_offset: usize,
    display_offset: usize,
    width: usize,
}

impl SourceLine {
    /// The one-based line number in the source file.
    pub(crate) fn line_number(&self) -> usize {
        self.line_number
    }
    /// The source content, without its line ending.
    pub(crate) fn text(&self) -> &SharedString {
        &self.text
    }
    /// The content with tabs expanded, as painted.
    pub(crate) fn display(&self) -> &SharedString {
        &self.display
    }
    /// Display ranges painted as separate text runs, bounding shaping cost.
    pub(crate) fn chunk_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.chunks.iter().map(|chunk| chunk.range.clone())
    }
    /// The index of the paired line on the other side, if any.
    pub(crate) fn counterpart(&self) -> Option<usize> {
        self.counterpart
    }
    /// The byte range in the side's source, including the line ending.
    pub(crate) fn source(&self) -> Range<usize> {
        self.source.clone()
    }
    /// The source offset where the line ending begins.
    pub(crate) fn content_end(&self) -> usize {
        self.content_end
    }

    /// Reuses the retained text for a projected chunk. The fallback supports
    /// valid partial ranges without making ordinary rendering allocate.
    pub(crate) fn chunk_text(&self, range: &Range<usize>) -> SharedString {
        if range.start == 0 && range.end == self.display.len() {
            return self.display.clone();
        }
        let ix = self
            .chunks
            .partition_point(|chunk| chunk.range.start < range.start);
        if let Some(chunk) = self.chunks.get(ix)
            && chunk.range == *range
        {
            return chunk.text.clone();
        }
        self.display
            .get(range.clone())
            .map_or_else(SharedString::default, SharedString::from)
    }

    pub(crate) fn source_offset(&self, display_offset: usize) -> usize {
        let display_offset = display_offset.min(self.display.len());
        let preceding = self
            .display_tabs
            .partition_point(|tab| tab.display_offset <= display_offset);
        let Some(tab) = preceding.checked_sub(1).map(|ix| &self.display_tabs[ix]) else {
            return display_offset;
        };
        if display_offset < tab.display_offset + tab.width {
            tab.source_offset
        } else {
            display_offset - (tab.display_offset + tab.width - tab.source_offset - 1)
        }
    }
    pub(crate) fn display_range(&self, source: Range<usize>) -> Range<usize> {
        self.display_offset(source.start)..self.display_offset(source.end)
    }

    fn display_offset(&self, source_offset: usize) -> usize {
        let source_offset = source_offset.min(self.text.len());
        let preceding = self
            .display_tabs
            .partition_point(|tab| tab.source_offset < source_offset);
        let expansion = preceding.checked_sub(1).map_or(0, |ix| {
            let tab = &self.display_tabs[ix];
            tab.display_offset + tab.width - tab.source_offset - 1
        });
        source_offset + expansion
    }
}

/// One row of the side-by-side alignment: a line on either or both sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinePair {
    original: Option<usize>,
    modified: Option<usize>,
    changed: bool,
}

impl LinePair {
    pub(crate) fn new(original: Option<usize>, modified: Option<usize>, changed: bool) -> Self {
        Self {
            original,
            modified,
            changed,
        }
    }
    pub(crate) fn original(&self) -> Option<usize> {
        self.original
    }
    pub(crate) fn modified(&self) -> Option<usize> {
        self.modified
    }
    pub(crate) fn is_changed(&self) -> bool {
        self.changed
    }
}

/// One `@@` hunk: its pairs and the source lines it supplies on each side.
#[derive(Clone, Debug)]
pub(crate) struct Hunk {
    pairs: Range<usize>,
    original: Range<usize>,
    modified: Range<usize>,
    label: SharedString,
    /// The one-based original lines the hunk supplies.
    original_lines: Range<usize>,
    /// Whether the hunk ended before its declared line counts.
    truncated: bool,
}

impl Hunk {
    pub(crate) fn new(
        pairs: Range<usize>,
        original: Range<usize>,
        modified: Range<usize>,
        label: SharedString,
        original_lines: Range<usize>,
        truncated: bool,
    ) -> Self {
        Self {
            pairs,
            original,
            modified,
            label,
            original_lines,
            truncated,
        }
    }
    /// Original lines between the previous hunk, or the file start, and this
    /// one that the patch does not supply.
    pub(crate) fn hidden_lines_before(&self, previous: Option<&Hunk>) -> usize {
        let after = previous.map_or(1, |previous| previous.original_lines.end);
        self.original_lines.start.saturating_sub(after)
    }
    pub(crate) fn pairs(&self) -> Range<usize> {
        self.pairs.clone()
    }
    pub(crate) fn lines(&self, side: DiffSide) -> Range<usize> {
        match side {
            DiffSide::Original => self.original.clone(),
            DiffSide::Modified => self.modified.clone(),
        }
    }
    /// The `@@` header line.
    pub(crate) fn label(&self) -> &SharedString {
        &self.label
    }
    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// One side of a file: its path, unless the side is missing such as
/// `/dev/null`, and the source fragments the patch supplies.
#[derive(Default)]
pub(crate) struct FileSide {
    path: Option<SharedString>,
    source: SharedString,
    lines: Vec<SourceLine>,
}

impl FileSide {
    /// Builds a side from its concatenated source and each line's number and
    /// byte range in it.
    pub(crate) fn new(
        path: Option<SharedString>,
        source: String,
        lines: impl IntoIterator<Item = (usize, Range<usize>)>,
    ) -> Self {
        let lines = lines
            .into_iter()
            .map(|(line_number, range)| source_line(&source, range, line_number))
            .collect();
        Self {
            path,
            source: source.into(),
            lines,
        }
    }
}

/// How a file changed, in Git's status terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiffFileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    /// A whole file shown for reference, from [`DiffFile::unchanged`].
    Unchanged,
    /// A working file with conflict markers, from [`DiffFile::parse_conflicts`].
    Conflicted,
}

/// One line of the patch body in patch order: an unchanged line on both
/// sides, or a deleted or added line on one side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PatchLine {
    original: Option<usize>,
    modified: Option<usize>,
}

impl PatchLine {
    pub(crate) fn line(&self, side: DiffSide) -> Option<usize> {
        match side {
            DiffSide::Original => self.original,
            DiffSide::Modified => self.modified,
        }
    }
}

struct DocumentInner {
    path: SharedString,
    status: DiffFileStatus,
    original: FileSide,
    modified: FileSide,
    pairs: Vec<LinePair>,
    hunks: Vec<Hunk>,
    additions: usize,
    deletions: usize,
    extended_headers: Vec<SharedString>,
    binary: bool,
    /// Width of the widest line number, in digits.
    line_number_digits: usize,
    /// The patch body in order; each side's lines map back into it.
    patch_lines: Vec<PatchLine>,
    patch_ixs: [Vec<usize>; 2],
    conflicts: Vec<DiffConflict>,
}

/// One changed file parsed from a unified or Git diff.
///
/// Only source fragments present in the patch are retained; unavailable
/// context is never reconstructed or compared. Cloning is cheap.
#[derive(Clone)]
pub struct DiffFile {
    inner: Arc<DocumentInner>,
    language: Option<SharedString>,
}

impl DiffFile {
    /// Parses every file in a unified or Git diff, in patch order, without
    /// computing differences. Syntax emphasis is prepared later by
    /// [`super::DiffState`] on a background thread.
    pub fn parse(patch: &str) -> Result<Vec<Self>, DiffParseError> {
        super::parser::parse(patch, DiffParseOptions::default())
    }

    /// Parses like [`DiffFile::parse`], applying the relaxations `options`
    /// enables. Malformed headers, invalid paths and unordered hunks are still
    /// errors.
    pub fn parse_with(patch: &str, options: DiffParseOptions) -> Result<Vec<Self>, DiffParseError> {
        super::parser::parse(patch, options)
    }

    /// Assembles a parsed file, pairing counterpart lines and deriving its
    /// display path, change counts and line-number width.
    pub(crate) fn new(
        status: DiffFileStatus,
        mut original: FileSide,
        mut modified: FileSide,
        pairs: Vec<LinePair>,
        hunks: Vec<Hunk>,
        extended_headers: Vec<SharedString>,
        binary: bool,
    ) -> Self {
        let (mut additions, mut deletions) = (0, 0);
        for pair in &pairs {
            // Conflict lines are unresolved alternatives, not additions.
            if pair.changed && status != DiffFileStatus::Conflicted {
                additions += usize::from(pair.modified.is_some());
                deletions += usize::from(pair.original.is_some());
            }
            if let (Some(old), Some(new)) = (pair.original, pair.modified) {
                original.lines[old].counterpart = Some(new);
                modified.lines[new].counterpart = Some(old);
            }
        }
        let widest = [&original, &modified]
            .iter()
            .filter_map(|side| side.lines.last())
            .map(|line| line.line_number)
            .max()
            .unwrap_or(1);
        let path = modified
            .path
            .clone()
            .or_else(|| original.path.clone())
            .unwrap_or_default();
        let (patch_lines, patch_ixs) = patch_order(&pairs, &hunks, &original, &modified);
        Self {
            inner: Arc::new(DocumentInner {
                path,
                status,
                original,
                modified,
                pairs,
                hunks,
                additions,
                deletions,
                extended_headers,
                binary,
                line_number_digits: widest.max(1).ilog10() as usize + 1,
                patch_lines,
                patch_ixs,
                conflicts: Vec::new(),
            }),
            language: None,
        }
    }

    /// Overrides the syntax language detected from the path, using a
    /// highlighter language name such as `rust` or `javascript`.
    pub fn with_language(mut self, language: impl Into<SharedString>) -> Self {
        self.language = Some(language.into());
        self
    }
    /// The syntax language: the override, or the one detected from the path.
    pub fn language(&self) -> SharedString {
        self.language
            .clone()
            .unwrap_or_else(|| super::presentation::detected_language(self.path()))
    }
    pub(crate) fn language_override(&self) -> Option<&SharedString> {
        self.language.as_ref()
    }
    /// How the file changed.
    pub fn status(&self) -> DiffFileStatus {
        self.inner.status
    }

    /// The path to show for this file: the modified path, or the original path
    /// of a deleted file.
    pub fn path(&self) -> &SharedString {
        &self.inner.path
    }
    /// The original path, or `None` for an added file.
    pub fn original_path(&self) -> Option<&SharedString> {
        self.inner.original.path.as_ref()
    }
    /// The modified path, or `None` for a deleted file.
    pub fn modified_path(&self) -> Option<&SharedString> {
        self.inner.modified.path.as_ref()
    }
    /// Git extended header lines, such as file modes, similarity, renames
    /// and the binary marker, as they appear in the patch.
    pub fn extended_headers(&self) -> &[SharedString] {
        &self.inner.extended_headers
    }
    /// Whether the patch reports binary contents rather than textual hunks.
    pub fn is_binary(&self) -> bool {
        self.inner.binary
    }
    /// Whether a hunk ended before its declared line counts, which only
    /// [`DiffParseOptions::truncated_hunks`] accepts.
    pub fn is_truncated(&self) -> bool {
        self.inner.hunks.iter().any(Hunk::is_truncated)
    }
    pub fn additions(&self) -> usize {
        self.inner.additions
    }
    pub fn deletions(&self) -> usize {
        self.inner.deletions
    }
    /// Whether the file changes, including an added or deleted empty file and
    /// metadata-only changes such as a mode change or pure rename.
    pub fn has_changes(&self) -> bool {
        self.additions() > 0
            || self.deletions() > 0
            || !self.inner.extended_headers.is_empty()
            || self.inner.binary
            || self.original_path().is_none()
            || self.modified_path().is_none()
            || self.status() == DiffFileStatus::Conflicted
    }

    /// Whether the file has one column in every display mode.
    pub(crate) fn is_single_column(&self) -> bool {
        matches!(
            self.status(),
            DiffFileStatus::Unchanged | DiffFileStatus::Conflicted
        )
    }
    /// Merge conflicts in working-file order; slice indices identify resolutions.
    pub fn conflicts(&self) -> &[DiffConflict] {
        &self.inner.conflicts
    }
    pub(crate) fn with_conflicts(mut self, conflicts: Vec<DiffConflict>) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("a newly built file is not shared")
            .conflicts = conflicts;
        self
    }

    pub(crate) fn lines_count(&self, side: DiffSide) -> usize {
        self.lines(side).len()
    }

    /// Copies patch-provided lines. Unavailable context is omitted, without
    /// inventing source lines.
    pub(crate) fn text_for_lines(&self, side: DiffSide, start: usize, end: usize) -> String {
        let lines = self.lines(side);
        let start_ix = lines.partition_point(|line| line.line_number < start);
        let end_ix = lines.partition_point(|line| line.line_number <= end);
        let selected = &lines[start_ix..end_ix];
        let (Some(first), Some(last)) = (selected.first(), selected.last()) else {
            return String::new();
        };
        self.source(side)[first.source.start..last.source.end].to_owned()
    }

    /// The one-based line number of the line at `index` on `side`.
    pub(crate) fn line_number(&self, side: DiffSide, index: usize) -> usize {
        self.lines(side)[index].line_number
    }
    pub(crate) fn line_index(&self, side: DiffSide, line: usize) -> Option<usize> {
        self.lines(side)
            .binary_search_by_key(&line, |source| source.line_number)
            .ok()
    }

    pub(crate) fn pairs(&self) -> &[LinePair] {
        &self.inner.pairs
    }
    pub(crate) fn hunks(&self) -> &[Hunk] {
        &self.inner.hunks
    }
    /// Width of the widest supplied line number, in digits.
    pub(crate) fn line_number_digits(&self) -> usize {
        self.inner.line_number_digits
    }

    pub(crate) fn patch_lines(&self) -> &[PatchLine] {
        &self.inner.patch_lines
    }
    /// The position of a side's line in [`Self::patch_lines`].
    pub(crate) fn patch_ix(&self, side: DiffSide, ix: usize) -> usize {
        self.inner.patch_ixs[side as usize][ix]
    }

    pub(crate) fn side_path(&self, side: DiffSide) -> Option<&SharedString> {
        self.side(side).path.as_ref()
    }

    fn side(&self, side: DiffSide) -> &FileSide {
        match side {
            DiffSide::Original => &self.inner.original,
            DiffSide::Modified => &self.inner.modified,
        }
    }
    pub(crate) fn lines(&self, side: DiffSide) -> &[SourceLine] {
        &self.side(side).lines
    }
    pub(crate) fn source(&self, side: DiffSide) -> &str {
        &self.side(side).source
    }
}

/// Orders the patch body as Git writes it: within each run of changes, every
/// deletion precedes every addition. Runs never cross hunk boundaries.
fn patch_order(
    pairs: &[LinePair],
    hunks: &[Hunk],
    original: &FileSide,
    modified: &FileSide,
) -> (Vec<PatchLine>, [Vec<usize>; 2]) {
    let mut lines = Vec::with_capacity(pairs.len());
    let mut ixs = [vec![0; original.lines.len()], vec![0; modified.lines.len()]];
    let ranges = if hunks.is_empty() {
        vec![0..pairs.len()]
    } else {
        hunks.iter().map(Hunk::pairs).collect()
    };
    for range in ranges {
        let mut ix = range.start;
        while ix < range.end {
            if !pairs[ix].changed {
                let pair = pairs[ix];
                lines.push(PatchLine {
                    original: pair.original,
                    modified: pair.modified,
                });
                ix += 1;
                continue;
            }
            let start = ix;
            while ix < range.end && pairs[ix].changed {
                ix += 1;
            }
            for pair in &pairs[start..ix] {
                if let Some(line) = pair.original {
                    lines.push(PatchLine {
                        original: Some(line),
                        modified: None,
                    });
                }
            }
            for pair in &pairs[start..ix] {
                if let Some(line) = pair.modified {
                    lines.push(PatchLine {
                        original: None,
                        modified: Some(line),
                    });
                }
            }
        }
    }
    for (patch_ix, line) in lines.iter().enumerate() {
        if let Some(ix) = line.original {
            ixs[0][ix] = patch_ix;
        }
        if let Some(ix) = line.modified {
            ixs[1][ix] = patch_ix;
        }
    }
    (lines, ixs)
}

/// Builds the display form of one source line from its range in `source`.
fn source_line(source: &str, range: Range<usize>, line_number: usize) -> SourceLine {
    let raw = &source[range.clone()];
    let content = match raw.strip_suffix('\n') {
        Some(content) => content.strip_suffix('\r').unwrap_or(content),
        None => raw,
    };
    let text = SharedString::from(content.to_owned());
    let (display, display_tabs): (SharedString, Vec<DisplayTab>) = if content.contains('\t') {
        let mut display = String::with_capacity(content.len());
        let mut display_tabs = Vec::new();
        let mut column = 0;
        for (ix, ch) in content.char_indices() {
            if ch == '\t' {
                let spaces = 4 - column % 4;
                display_tabs.push(DisplayTab {
                    source_offset: ix,
                    display_offset: display.len(),
                    width: spaces,
                });
                for _ in 0..spaces {
                    display.push(' ');
                }
                column += spaces;
            } else {
                display.push(ch);
                column += 1;
            }
        }
        (display.into(), display_tabs)
    } else {
        (text.clone(), Vec::new())
    };
    let chunks = display_chunks(&display)
        .into_iter()
        .map(|range| {
            let text = if range.start == 0 && range.end == display.len() {
                display.clone()
            } else {
                SharedString::from(display[range.clone()].to_owned())
            };
            DisplayChunk { range, text }
        })
        .collect();
    SourceLine {
        line_number,
        text,
        display,
        chunks,
        counterpart: None,
        display_tabs,
        content_end: range.start + content.len(),
        source: range,
    }
}

/// Bound native selection projection while preserving grapheme boundaries.
/// Exceptionally large clusters split only at UTF-8 boundaries.
pub(crate) fn display_chunks(text: &str) -> SmallVec<[Range<usize>; 1]> {
    const MAX_BYTES: usize = 512;
    if text.len() <= MAX_BYTES {
        return smallvec![0..text.len()];
    }
    let mut chunks = SmallVec::new();
    let mut start = 0;
    let mut end = 0;
    for (ix, grapheme) in text.grapheme_indices(true) {
        if ix + grapheme.len() - start > MAX_BYTES && start < ix {
            chunks.push(start..ix);
            start = ix;
        }
        if grapheme.len() > MAX_BYTES {
            for (relative, character) in grapheme.char_indices() {
                let offset = ix + relative;
                if offset + character.len_utf8() - start > MAX_BYTES {
                    chunks.push(start..offset);
                    start = offset;
                }
            }
        }
        end = ix + grapheme.len();
    }
    if start < end {
        chunks.push(start..end);
    }
    chunks
}
