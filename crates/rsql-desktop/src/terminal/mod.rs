//! Worker-owned terminal emulation and native PTY session ownership.

mod events;
mod handler;
pub(crate) mod input;
mod limits;
mod palette;
mod selection;
pub mod session;
mod snapshot;

use alacritty_terminal::Term;
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Osc52};
use alacritty_terminal::vte::ansi::{CursorStyle, Processor};
use std::time::Instant;

pub use alacritty_terminal::grid::Scroll;
pub use alacritty_terminal::index::Side;
pub use alacritty_terminal::selection::SelectionType;
pub use alacritty_terminal::term::TermMode;
pub use alacritty_terminal::term::cell::{Flags as CellFlags, Hyperlink};
pub use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
pub use events::{MAX_REPLY_BYTES, MAX_REPLY_MESSAGES};
pub use limits::*;
pub use selection::DisplayedPoint;
pub(crate) use selection::DisplayedRows;
pub use snapshot::*;

pub(crate) const HISTORY_LINES: usize = 5000;

/// Own this model on a worker. Snapshots copy visible cells, never scrollback.
/// The library owns the parser, both screens, scrollback and selection.
pub struct TerminalModel {
    term: Term<events::Events>,
    parser: Processor,
    events: events::Events,
    palette: [Rgb; alacritty_terminal::term::color::COUNT],
    cell_size: (u16, u16),
    revision: u64,
    mouse_generation: u64,
    // Cumulative viewport movement requested by the user, excluding child output.
    user_scroll: i64,
}

impl TerminalModel {
    /// Normalize transient zero sizes to two columns/one row so wide cells fit.
    pub fn new(columns: usize, rows: usize) -> Self {
        Self::with_limits(columns, rows, TerminalLimits::default())
    }

    /// Allow stricter limits; requested values are capped at production maxima.
    pub fn with_limits(columns: usize, rows: usize, limits: TerminalLimits) -> Self {
        let size = Size::new(columns, rows);
        let events = events::Events::default();
        let config = Config {
            scrolling_history: HISTORY_LINES,
            combining_limit: Some(limits.combining_scalars.min(MAX_COMBINING_SCALARS)),
            default_cursor_style: CursorStyle {
                shape: CursorShape::Beam,
                blinking: true,
            },
            osc52: Osc52::Disabled,
            kitty_keyboard: false,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &size, events.clone()),
            parser: Processor::with_osc_limit(limits.osc_bytes.min(MAX_OSC_BYTES)),
            events,
            palette: palette::colors(TerminalTheme::Light),
            cell_size: (0, 0),
            revision: 0,
            mouse_generation: 0,
            user_scroll: 0,
        }
    }

    /// Admit at most 64 KiB, returning at a library atomic sync flush too.
    /// `processed` measures newly admitted bytes, never the buffered decode work.
    /// On Yield, or whenever `synchronized_flush` is true, the owner must yield
    /// and check close before resuming at `accepted`. On backpressure drain
    /// replies first. A fault requires closing the session and a fresh model;
    /// the triggering byte/batch may have mutated state, so never replay it.
    pub fn feed(&mut self, bytes: &[u8]) -> FeedOutcome {
        if let Some(fault) = self.events.lock().fault {
            return FeedOutcome {
                synchronized_flush: false,
                accepted: 0,
                processed: 0,
                status: FeedStatus::Fault(fault),
            };
        }
        let size = self.window_size();
        for (index, byte) in bytes.iter().take(MAX_FEED_BYTES).enumerate() {
            if !self.events.lock().has_headroom() {
                return FeedOutcome {
                    synchronized_flush: false,
                    accepted: index,
                    processed: index,
                    status: FeedStatus::Backpressure,
                };
            }
            let buffered = self.parser.sync_bytes_count();
            let mut handler = handler::QueryTimeHandler {
                term: &mut self.term,
                mouse_generation: &mut self.mouse_generation,
                events: &self.events,
                palette: &self.palette,
                size,
            };
            self.parser
                .advance(&mut handler, std::slice::from_ref(byte));
            let synchronized_flush = buffered > 0 && self.parser.sync_bytes_count() <= buffered;
            self.revision = self.revision.wrapping_add(1);
            if let Some(fault) = self.resolve_events(size) {
                return FeedOutcome {
                    synchronized_flush,
                    accepted: index,
                    processed: index + 1,
                    status: FeedStatus::Fault(fault),
                };
            }
            if synchronized_flush {
                return FeedOutcome {
                    synchronized_flush,
                    accepted: index + 1,
                    processed: index + 1,
                    status: if index + 1 < bytes.len() {
                        FeedStatus::Yield
                    } else {
                        FeedStatus::Ready
                    },
                };
            }
        }
        FeedOutcome {
            synchronized_flush: false,
            accepted: bytes.len().min(MAX_FEED_BYTES),
            processed: bytes.len().min(MAX_FEED_BYTES),
            status: if bytes.len() > MAX_FEED_BYTES {
                FeedStatus::Yield
            } else {
                FeedStatus::Ready
            },
        }
    }

    pub fn resize(&mut self, columns: usize, rows: usize) {
        let size = Size::new(columns, rows);
        if self.term.columns() != size.columns || self.term.screen_lines() != size.rows {
            self.term.resize(size);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        let previous = self.term.grid().display_offset();
        self.term.scroll_display(scroll);
        let moved = self.term.grid().display_offset() as i64 - previous as i64;
        if moved != 0 {
            self.user_scroll = self.user_scroll.wrapping_add(moved);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Unknown pixel size is zero until the renderer provides measured cells.
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        self.cell_size = (width, height);
    }

    pub fn set_theme(&mut self, theme: TerminalTheme) {
        let palette = palette::colors(theme);
        if self.palette != palette {
            self.palette = palette;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn take_replies(&mut self) -> Vec<Vec<u8>> {
        self.events.lock().take_replies()
    }

    /// The later owner schedules this deadline; the model starts no timer/task.
    pub fn synchronized_update_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    /// Pending synchronized bytes, bounded by the pinned 2 MiB library buffer.
    pub fn buffered_synchronized_bytes(&self) -> usize {
        self.parser.sync_bytes_count()
    }

    /// Atomically decode the pending library buffer, admitting zero new bytes.
    /// This can decode up to 2 MiB; the owner must yield/check close immediately
    /// afterward. A fault requires a fresh session, even without new input.
    /// The model starts no timer and does not measure flush latency.
    pub fn flush_synchronized_update(&mut self) -> FeedStatus {
        if let Some(fault) = self.events.lock().fault {
            return FeedStatus::Fault(fault);
        }
        if !self.events.lock().has_headroom() {
            return FeedStatus::Backpressure;
        }
        if self.synchronized_update_deadline().is_none() {
            return FeedStatus::Ready;
        }
        let size = self.window_size();
        let mut handler = handler::QueryTimeHandler {
            term: &mut self.term,
            mouse_generation: &mut self.mouse_generation,
            events: &self.events,
            palette: &self.palette,
            size,
        };
        self.parser.stop_sync(&mut handler);
        self.revision = self.revision.wrapping_add(1);
        self.resolve_events(size)
            .map_or(FeedStatus::Ready, FeedStatus::Fault)
    }

    fn window_size(&self) -> WindowSize {
        let num_cols = self.term.columns() as u16;
        let num_lines = self.term.screen_lines() as u16;
        WindowSize {
            num_cols,
            num_lines,
            cell_width: self.cell_size.0.min(u16::MAX / num_cols),
            cell_height: self.cell_size.1.min(u16::MAX / num_lines),
        }
    }

    fn resolve_events(&self, size: WindowSize) -> Option<TerminalFault> {
        let mut events = self.events.lock();
        if let Some(limit) = self.parser.osc_limit_exceeded() {
            events
                .fault
                .get_or_insert(TerminalFault::InputLimit(InputLimit::OscBytes { limit }));
        }
        events.resolve(self.term.colors(), &self.palette, size);
        events.fault
    }
}

struct Size {
    columns: usize,
    rows: usize,
}
impl Size {
    fn new(columns: usize, rows: usize) -> Self {
        Self {
            columns: columns.clamp(2, u16::MAX as usize),
            rows: rows.clamp(1, u16::MAX as usize),
        }
    }
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}
