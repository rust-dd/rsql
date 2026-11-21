use super::{CellFlags, Color, CursorShape, Hyperlink, Rgb, TermMode};
use alacritty_terminal::selection::SelectionRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCell {
    /// Display text plus combining marks; tab cells render as a space.
    /// Copying uses the library selection, which retains the original tab.
    pub text: String,
    pub foreground: Color,
    pub background: Color,
    pub underline_color: Option<Color>,
    pub flags: CellFlags,
    /// Shared owned library payload, retained across snapshots/model changes.
    /// Untrusted data; consumers must never open it without a user action.
    pub hyperlink: Option<Hyperlink>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCursor {
    pub row: usize,
    pub column: usize,
    pub shape: CursorShape,
    pub blinking: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Owned row-major viewport cells and fixed-size render metadata, not history.
pub struct TerminalSnapshot {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<TerminalCell>,
    pub cursor: TerminalCursor,
    pub mode: TermMode,
    pub history_size: usize,
    pub scroll_offset: usize,
    pub selection: Option<SelectionRange>,
    pub colors: [Rgb; 269],
    /// Library-normalized title data; never a command or trusted markup.
    pub title: Option<String>,
    pub revision: u64,
    pub fault: Option<TerminalFault>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalTheme {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Fatal protocol failure. Keep queued replies for diagnostics, close the
/// owning session and create a new model; the parser cannot be replayed safely.
pub enum TerminalFault {
    ReplyOverflow { rejected_replies: usize },
    InputLimit(super::InputLimit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Backpressure is recoverable after draining replies; a fault is not.
pub enum FeedStatus {
    Ready,
    Backpressure,
    /// Yield and check session closure before resuming the exact suffix.
    Yield,
    Fault(TerminalFault),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct FeedOutcome {
    /// A buffered atomic flush occurred. Yield/check close before the next call,
    /// even when status is Ready. Flushed bytes are not included in `processed`.
    pub synchronized_flush: bool,
    /// Prefix accepted before backpressure or the byte causing a fatal fault.
    pub accepted: usize,
    /// Bytes handed to the stateful parser, including a fault-triggering byte.
    /// This can exceed `accepted`; never replay a faulted model. It excludes
    /// previously buffered synchronized bytes decoded atomically by this call.
    pub processed: usize,
    pub status: FeedStatus,
}

impl super::TerminalModel {
    pub fn snapshot(&self) -> TerminalSnapshot {
        use alacritty_terminal::grid::Dimensions;
        let content = self.term.renderable_content();
        let rows = self.term.screen_lines();
        let cursor_row = (content.cursor.point.line.0 as usize) + content.display_offset;
        let cursor = TerminalCursor {
            row: cursor_row,
            column: content.cursor.point.column.0,
            shape: content.cursor.shape,
            blinking: self.term.cursor_style().blinking,
            visible: cursor_row < rows && content.cursor.shape != CursorShape::Hidden,
        };
        let cells = content
            .display_iter
            .map(|indexed| {
                let display_char = if indexed.cell.c == '\t' {
                    ' '
                } else {
                    indexed.cell.c
                };
                let mut text = display_char.to_string();
                if let Some(combining) = indexed.cell.zerowidth() {
                    text.extend(combining);
                }
                let selected = content.selection.as_ref().is_some_and(|range| {
                    range.contains_cell(&indexed, content.cursor.point, content.cursor.shape)
                });
                TerminalCell {
                    text,
                    foreground: indexed.cell.fg,
                    background: indexed.cell.bg,
                    underline_color: indexed.cell.underline_color(),
                    flags: indexed.cell.flags,
                    hyperlink: indexed.cell.hyperlink(),
                    selected,
                }
            })
            .collect();
        let colors =
            std::array::from_fn(|index| content.colors[index].unwrap_or(self.palette[index]));
        let events = self.events.lock();
        TerminalSnapshot {
            columns: self.term.columns(),
            rows,
            cells,
            cursor,
            mode: content.mode,
            history_size: self.term.history_size(),
            scroll_offset: content.display_offset,
            selection: content.selection,
            colors,
            title: events.title.clone(),
            revision: self.revision,
            fault: events.fault,
        }
    }
}
