//! Files outside a patch: a whole source shown for reference, and a working
//! file containing Git conflict markers.
use std::ops::Range;

use gpui::SharedString;

use super::{
    DiffFile, DiffFileStatus, DiffParseError,
    document::{FileSide, LinePair},
};

/// How the user resolved a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiffConflictResolution {
    /// Keep the current change, between `<<<<<<<` and `=======`.
    Current,
    /// Keep the incoming change, between `=======` and `>>>>>>>`.
    Incoming,
    /// Keep the current change followed by the incoming one.
    Both,
}

/// Which part of a conflict a row introduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConflictPart {
    Current,
    Base,
    Incoming,
}

/// A parsed merge conflict in a working file.
///
/// Source coordinates are one-based, half-open ranges, allowing empty parts.
/// Returned by [`DiffFile::conflicts`] in source order; its slice index is the
/// index accepted by [`super::DiffState::resolve_conflict`]. Fields are private
/// and only the parser constructs these snapshots.
#[derive(Clone, Debug)]
pub struct DiffConflict {
    lines: Range<usize>,
    current_lines: Range<usize>,
    base_lines: Option<Range<usize>>,
    incoming_lines: Range<usize>,
    current: Range<usize>,
    base: Option<Range<usize>>,
    incoming: Range<usize>,
    current_label: SharedString,
    base_label: SharedString,
    incoming_label: SharedString,
}

impl DiffConflict {
    /// One-based working-file lines, including markers, with an exclusive end.
    pub fn lines(&self) -> Range<usize> {
        self.lines.clone()
    }
    /// Current-side source lines, with an exclusive end; may be empty.
    pub fn current_lines(&self) -> Range<usize> {
        self.current_lines.clone()
    }
    /// Base source lines for diff3, with an exclusive end; may be empty.
    pub fn base_lines(&self) -> Option<Range<usize>> {
        self.base_lines.clone()
    }
    /// Incoming-side source lines, with an exclusive end; may be empty.
    pub fn incoming_lines(&self) -> Range<usize> {
        self.incoming_lines.clone()
    }
    /// The label from the opening marker.
    pub fn current_label(&self) -> &SharedString {
        &self.current_label
    }
    /// The base marker label, or None for a two-way conflict.
    pub fn base_label(&self) -> Option<&SharedString> {
        self.base.as_ref().map(|_| &self.base_label)
    }
    /// The label from the closing marker.
    pub fn incoming_label(&self) -> &SharedString {
        &self.incoming_label
    }
    /// Every line of the region, in file order.
    pub(crate) fn source_lines(&self) -> Range<usize> {
        self.current.start..self.incoming.end
    }
    pub(crate) fn part(&self, part: ConflictPart) -> Option<Range<usize>> {
        match part {
            ConflictPart::Current => Some(self.current.clone()),
            ConflictPart::Base => self.base.clone(),
            ConflictPart::Incoming => Some(self.incoming.clone()),
        }
    }
    pub(crate) fn label(&self, part: ConflictPart) -> &SharedString {
        match part {
            ConflictPart::Current => &self.current_label,
            ConflictPart::Base => &self.base_label,
            ConflictPart::Incoming => &self.incoming_label,
        }
    }
    /// The part a line of the region belongs to.
    pub(crate) fn part_of(&self, ix: usize) -> Option<ConflictPart> {
        [
            ConflictPart::Current,
            ConflictPart::Base,
            ConflictPart::Incoming,
        ]
        .into_iter()
        .find(|part| self.part(*part).is_some_and(|range| range.contains(&ix)))
    }
    /// The lines a resolution keeps.
    pub(crate) fn kept(&self, resolution: DiffConflictResolution) -> Vec<Range<usize>> {
        match resolution {
            DiffConflictResolution::Current => vec![self.current.clone()],
            DiffConflictResolution::Incoming => vec![self.incoming.clone()],
            DiffConflictResolution::Both => vec![self.current.clone(), self.incoming.clone()],
        }
    }
}

/// Splits `text` into lines and their byte ranges, keeping line endings.
fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut offset = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let range = offset..offset + line.len();
            offset = range.end;
            range
        })
        .collect()
}

impl DiffFile {
    /// A file shown in full, for reference, without changes. It appears as a
    /// single column in every display mode.
    pub fn unchanged(path: impl Into<SharedString>, text: &str) -> Self {
        let path = path.into();
        let ranges = line_ranges(text);
        let pairs = (0..ranges.len())
            .map(|ix| LinePair::new(None, Some(ix), false))
            .collect();
        let lines = ranges
            .into_iter()
            .enumerate()
            .map(|(ix, range)| (ix + 1, range));
        DiffFile::new(
            DiffFileStatus::Unchanged,
            FileSide::new(Some(path.clone()), String::new(), []),
            FileSide::new(Some(path), text.to_owned(), lines),
            pairs,
            Vec::new(),
            Vec::new(),
            false,
        )
    }

    /// Parses a working file containing Git conflict markers, including the
    /// `|||||||` base section of `diff3` style. Lines keep their numbers in
    /// the working file; marker lines are shown as conflict headings instead.
    pub fn parse_conflicts(
        path: impl Into<SharedString>,
        text: &str,
    ) -> Result<Self, DiffParseError> {
        let path = path.into();
        let mut source = String::new();
        let mut lines = Vec::new();
        let mut pairs = Vec::new();
        let mut conflicts = Vec::new();
        // The marker-delimited part being read, with the open region's ranges.
        let mut open: Option<(ConflictPart, DiffConflict)> = None;
        for (ix, range) in line_ranges(text).into_iter().enumerate() {
            let line = &text[range];
            let content = line.trim_end_matches(['\n', '\r']);
            let marker = |prefix: &str| {
                content
                    .strip_prefix(prefix)
                    .filter(|rest| rest.is_empty() || rest.starts_with(' '))
                    .map(|rest| SharedString::from(rest.trim().to_owned()))
            };
            let next = lines.len();
            let (opening, base, separator, closing) = (
                marker("<<<<<<<"),
                marker("|||||||"),
                marker("======="),
                marker(">>>>>>>"),
            );
            match open.as_mut() {
                None => {
                    if let Some(label) = opening {
                        open = Some((
                            ConflictPart::Current,
                            DiffConflict {
                                lines: ix + 1..ix + 1,
                                current_lines: ix + 2..ix + 2,
                                base_lines: None,
                                incoming_lines: ix + 2..ix + 2,
                                current: next..next,
                                base: None,
                                incoming: next..next,
                                current_label: label,
                                base_label: SharedString::default(),
                                incoming_label: SharedString::default(),
                            },
                        ));
                        continue;
                    }
                    if base.is_some() || closing.is_some() {
                        return Err(DiffParseError::new(
                            ix + 1,
                            "conflict marker outside a conflict",
                        ));
                    }
                }
                Some((part, conflict)) => {
                    if opening.is_some() {
                        return Err(DiffParseError::new(ix + 1, "nested conflict marker"));
                    }
                    match (*part, base, separator, closing) {
                        (ConflictPart::Current, Some(label), _, _) => {
                            conflict.current.end = next;
                            conflict.current_lines.end = ix + 1;
                            conflict.base_lines = Some(ix + 2..ix + 2);
                            conflict.base = Some(next..next);
                            conflict.base_label = label;
                            *part = ConflictPart::Base;
                            continue;
                        }
                        (ConflictPart::Current | ConflictPart::Base, _, Some(_), _) => {
                            if *part == ConflictPart::Current {
                                conflict.current.end = next;
                                conflict.current_lines.end = ix + 1;
                            } else if let Some(base) = &mut conflict.base {
                                base.end = next;
                                conflict.base_lines.as_mut().unwrap().end = ix + 1;
                            }
                            conflict.incoming = next..next;
                            conflict.incoming_lines = ix + 2..ix + 2;
                            *part = ConflictPart::Incoming;
                            continue;
                        }
                        (ConflictPart::Incoming, _, _, Some(label)) => {
                            conflict.incoming.end = next;
                            conflict.incoming_lines.end = ix + 1;
                            conflict.lines.end = ix + 2;
                            conflict.incoming_label = label;
                            if let Some((_, conflict)) = open.take() {
                                conflicts.push(conflict);
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
            }
            let start = source.len();
            source.push_str(line);
            lines.push((ix + 1, start..source.len()));
            pairs.push(LinePair::new(None, Some(next), open.is_some()));
        }
        if open.is_some() {
            return Err(DiffParseError::new(
                text.lines().count(),
                "unterminated conflict",
            ));
        }
        Ok(DiffFile::new(
            DiffFileStatus::Conflicted,
            FileSide::new(Some(path.clone()), String::new(), []),
            FileSide::new(Some(path), source, lines),
            pairs,
            Vec::new(),
            Vec::new(),
            false,
        )
        .with_conflicts(conflicts))
    }
}
